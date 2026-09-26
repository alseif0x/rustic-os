# SPDX-License-Identifier: Apache-2.0
"""Adopt a staged V7 file-server, then recover to the embedded service."""
import json
from pathlib import Path
import re
import tempfile
import time
import uuid

import environment
from .connection import Connection
from .machine import machine
from .v7_read import (
    BOOT_TIMEOUT,
    IDENTITY_REFUSAL,
    UART_BUDGET,
    _read_file,
    processes,
    stage_pair,
    version_text,
    volume_json,
)
from .v7_launch import manifest_facts


ROOT = environment.ROOT
FILE_SERVER_PROGRAM = 5
ADOPT_KIND = 40
ADOPT_REFUSAL = "adoption refused: manifest identity is not accepted for this storage launch"
SERVICE_LINE = re.compile(
    r"(?m)^files pid=(\d+) ([^;]+); files_source=(embedded|storage); "
    r"shell pid=(\d+); owner-policy id=(\d+)"
)
PROGRAM_IDS = {
    "supervisor": 0,
    "files": 1,
    "shell": 2,
    "utility": 3,
    "tasks": 4,
    "staged": FILE_SERVER_PROGRAM,
}


def _service_facts(uart):
    text = uart.command("services")
    match = SERVICE_LINE.search(text)
    if not match:
        raise AssertionError(f"unrecognized services reply: {text!r}")
    return {
        "files_pid": int(match[1]),
        "files_state": match[2],
        "files_source": match[3],
        "shell_pid": int(match[4]),
        "owner_policy": int(match[5]),
    }


def _pair(workspace, elf, manifest):
    return (
        workspace,
        elf["resource"],
        manifest["resource"],
        version_text(elf["version"]),
        version_text(manifest["version"]),
    )


def _stage(uart, pair, program):
    staged = stage_pair(uart, *pair)
    if staged["state"] != "staged":
        raise AssertionError(f"{program} pair was not staged: {staged}")
    pid = staged["pid"]
    row = processes(uart).get(pid)
    if row != {"state": "dormant", "program": "staged"}:
        raise AssertionError(f"staged {program} is not a dormant dynamic image: {row}")
    staged["process"] = row
    return staged


def _add_utility(volume_tool, data, workspace, elf, manifest):
    facts = manifest_facts(manifest.read_bytes())
    if facts["identity"] != "rustic.utility" or facts["artifact_sha256"] != environment.digest(elf):
        raise AssertionError("built utility manifest does not bind the utility ELF")
    added_elf = volume_json(
        volume_tool, "add7", data, workspace, "utility-adopt-test.elf", elf
    )
    added_manifest = volume_json(
        volume_tool, "add7", data, workspace, "utility-adopt-test.manifest", manifest
    )
    for added, path in ((added_elf, elf), (added_manifest, manifest)):
        item = added["file"]
        if item["size"] != path.stat().st_size or item["sha256"] != environment.digest(path):
            raise AssertionError(f"utility publication differs from {path.name}")
    if added_elf["workspace"] != added_manifest["workspace"]:
        raise AssertionError("utility pair was published into different workspaces")
    return added_elf["file"], added_manifest["file"], facts


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-adopt")
    output.mkdir(parents=True, exist_ok=True)
    for name in ("result.json", "terminal-v7-adopt.json", "serial.log", "qemu.log"):
        (output / name).unlink(missing_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    file_elf = ROOT / "target/native/file-server.elf"
    file_manifest = ROOT / "target/native/file-server.manifest"
    utility_elf = ROOT / "target/native/utility.elf"
    utility_manifest = ROOT / "target/native/utility.manifest"
    artifacts = metadata["native_applications"]
    for name, path, suffix in (
        ("file-server", file_elf, ".elf"),
        ("file-server", file_manifest, ".manifest"),
        ("utility", utility_elf, ".elf"),
        ("utility", utility_manifest, ".manifest"),
    ):
        if environment.digest(path) != artifacts[name][suffix]:
            raise RuntimeError(f"{path.name} differs from the terminal-v7 image build")

    started = time.monotonic()
    transcript = output / "serial.log"
    log = output / "qemu.log"
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-adopt-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-volume.raw"
            lineage = uuid.uuid4().hex
            seeded = volume_json(volume_tool, "seed7", data, lineage, file_elf, file_manifest)
            if seeded["lineage"] != lineage:
                raise AssertionError("host provisioner returned another lineage")
            utility_elf_ref, utility_manifest_ref, utility_facts = _add_utility(
                volume_tool,
                data,
                seeded["workspace"]["text"],
                utility_elf,
                utility_manifest,
            )
            file_refs = {
                "elf": {"workspace": seeded["workspace"]["text"], "resource": seeded["elf"]["resource"]},
                "manifest": {
                    "workspace": seeded["workspace"]["text"],
                    "resource": seeded["manifest"]["resource"],
                },
            }
            file_pair = _pair(seeded["workspace"]["text"], seeded["elf"], seeded["manifest"])
            utility_pair = _pair(
                seeded["workspace"]["text"], utility_elf_ref, utility_manifest_ref
            )
            report_before = volume_json(volume_tool, "report7", data)
            volume_sha_before = environment.digest(data)
            volume_bytes = data.stat().st_size

            sock = temporary / "uart.sock"
            with machine(image, data, f"unix:{sock},server=on,wait=off", log) as vm:
                uart = Connection(sock, vm, transcript, BOOT_TIMEOUT, output / "commands.jsonl", budget=UART_BUDGET)
                try:
                    uart.until()
                    initial_mount = uart.command("job-status 1")
                    if "kind=11 status=0" not in initial_mount:
                        raise AssertionError(f"initial embedded V7 mount failed: {initial_mount!r}")
                    before_adoption = _service_facts(uart)
                    if before_adoption["files_source"] != "embedded":
                        raise AssertionError(f"initial file service is not embedded: {before_adoption}")

                    staged_file = _stage(uart, file_pair, "file-server")
                    file_pid = staged_file["pid"]
                    start_refusal = uart.command(
                        f"start-staged {file_pid} exit", "error: start refused:"
                    )
                    if IDENTITY_REFUSAL not in start_refusal:
                        raise AssertionError(f"staged file-server start was not identity-refused: {start_refusal!r}")
                    if processes(uart).get(file_pid) != staged_file["process"]:
                        raise AssertionError("refused start-staged changed the dormant file-server")

                    adopted_text = uart.command(
                        f"adopt-files {file_pid}", f"files adopted pid={file_pid} source=storage"
                    )
                    if f"adopt-files ticks=" not in adopted_text:
                        raise AssertionError(f"adoption did not print its result timing: {adopted_text!r}")
                    adopted = _service_facts(uart)
                    if adopted["files_pid"] != file_pid or adopted["files_source"] != "storage":
                        raise AssertionError(f"services did not report the adopted file service: {adopted}")
                    file_process = processes(uart).get(file_pid)
                    if not file_process or file_process["program"] != "staged":
                        raise AssertionError(f"adopted PID is not reported as a dynamic image: {file_process}")
                    program_id = PROGRAM_IDS[file_process["program"]]
                    if program_id != FILE_SERVER_PROGRAM:
                        raise AssertionError(f"adopted PROCESS program is {program_id}, expected 5")
                    adopted_reads = [
                        _read_file(uart, file_refs["elf"], file_elf),
                        _read_file(uart, file_refs["manifest"], file_manifest),
                    ]

                    nonstaged_refusal = uart.command(
                        f"adopt-files {adopted['shell_pid']}", "error: service denied"
                    )
                    staged_utility = _stage(uart, utility_pair, "utility")
                    utility_pid = staged_utility["pid"]
                    utility_refusal = uart.command(
                        f"adopt-files {utility_pid}", f"error: {ADOPT_REFUSAL}"
                    )
                    if processes(uart).get(utility_pid) != staged_utility["process"]:
                        raise AssertionError("identity refusal changed the dormant utility")
                    after_utility_refusal = _service_facts(uart)
                    if (after_utility_refusal["files_pid"], after_utility_refusal["files_source"]) != (
                        file_pid,
                        "storage",
                    ):
                        raise AssertionError("utility identity refusal changed the running file service")
                    uart.command(f"kill {utility_pid}", "ok exit_kind=0 code=0")
                    uart.command(f"reap {utility_pid}", "ok exit_kind=3 code=0")

                    restart_text = uart.command(
                        "restart files", "files restarted; utility sessions revoked"
                    )
                    embedded = _service_facts(uart)
                    if embedded["files_source"] != "embedded" or embedded["files_pid"] in (0, file_pid):
                        raise AssertionError(f"restart did not restore the embedded service: {embedded}")
                    embedded_reads = [
                        _read_file(uart, file_refs["elf"], file_elf),
                        _read_file(uart, file_refs["manifest"], file_manifest),
                    ]
                    uart.send(b"exit\r")
                    uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
                    if vm.wait(timeout=10) != 33:
                        raise RuntimeError("unclean terminal-v7 adoption exit")
                    commands = uart.commands
                finally:
                    uart.close()

            serial = transcript.read_text(errors="replace")
            markers = ("RUSTIC PANIC", "RUSTIC FATAL", "RUSTIC EXCEPTION", "RUSTIC FAULT")
            seen_markers = [marker for marker in markers if marker in serial]
            if seen_markers:
                raise AssertionError(f"guest emitted failure markers: {seen_markers}")
            volume_sha_after = environment.digest(data)
            report_after = volume_json(volume_tool, "report7", data)
            if volume_sha_before != volume_sha_after or report_before != report_after:
                raise AssertionError("adoption or restart changed the V7 volume")

        evidence = {
            "verified": True,
            "mode": "terminal-v7-adopt",
            "image": {
                "build_id": metadata["build_id"],
                "image_sha256": metadata["image_sha256"],
                "kernel_sha256": metadata["kernel_sha256"],
            },
            "boot_count": 1,
            "adoption": {
                "pid": file_pid,
                "job_kind": ADOPT_KIND,
                "program_id": FILE_SERVER_PROGRAM,
                "services": adopted,
                "start_staged_refusal": IDENTITY_REFUSAL,
                "reads": adopted_reads,
            },
            "refusals": {
                "non_staged_pid": {"shell_pid": adopted["shell_pid"], "output": nonstaged_refusal.strip()},
                "utility_identity": {
                    "pid": utility_pid,
                    "identity": utility_facts["identity"],
                    "output": utility_refusal.strip(),
                    "dormant_after_refusal": True,
                },
            },
            "restart_embedded": {"services": embedded, "reads": embedded_reads, "output": restart_text.strip()},
            "volume": {
                "bytes": volume_bytes,
                "sha256_before": volume_sha_before,
                "sha256_after": volume_sha_after,
                "report_unchanged": True,
            },
            "commands": commands,
            "failure_markers": seen_markers,
        }
        (output / "terminal-v7-adopt.json").write_text(json.dumps(evidence, indent=1) + "\n")
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "terminal_v7_adopt": evidence,
        }
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print(
            f"V7 adopt acceptance: PID {file_pid} mounted from storage, then embedded restart; "
            f"volume sha256 unchanged after {commands} shell commands.",
            flush=True,
        )
        return result
    except Exception as error:
        (output / "result.json").write_text(
            json.dumps({"outcome": "failure", "error": str(error)}, separators=(",", ":")) + "\n"
        )
        raise
