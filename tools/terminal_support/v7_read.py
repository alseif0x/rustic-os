# SPDX-License-Identifier: Apache-2.0
"""Read a provisioned V7 application artifact in a disposable UEFI guest."""
import base64
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time
import uuid

import environment
from .connection import Connection
from .machine import machine
from .read_cases import read as read_range


ROOT = environment.ROOT
CHUNK_BYTES = 1024
BOOT_TIMEOUT = 300
STAGE_TIMEOUT = 200  # above the supervisor budget of 15,000 ticks
# Supervisor owner statuses from crates/abi/src/supervisor.rs (`stage`).
FILE_ERROR_BASE = 32
FILE_VERSION = 13
STAGE_KIND = 36
SUPERSEDED = 6
COPYING_PHASE = 2
PROCESS_ROW = re.compile(r"(?m)^(\d+) (\w+) (\d+) (\d+) (\d+) (\d+) (\S+)\r?$")


def version_text(value):
    return f"v_{value:016x}"


def stage_outcome(text):
    """Decode one `job-status` answer for a stage job; anything else is local failure."""
    text = text.replace("\r\n", "\n")
    pending = re.search(r"(?m)^job=(\d+) pending kind=(\d+) phase=(\d+) service=\d+ pending_io=\d+$", text)
    if pending:
        if int(pending[2]) != STAGE_KIND:
            raise ValueError("pending job is not a stage job")
        return {"job": int(pending[1]), "state": "pending", "phase": int(pending[3])}
    staged = re.search(
        r"(?m)^job=(\d+) complete kind=(\d+) status=0 staged pid=(\d+) "
        r"elf_version=(v_[0-9a-f]{16}) manifest_version=(v_[0-9a-f]{16}) state=dormant$",
        text,
    )
    if staged:
        if int(staged[2]) != STAGE_KIND or "\nerror:" in text:
            raise ValueError("ambiguous stage success")
        return {"job": int(staged[1]), "state": "staged", "pid": int(staged[3]),
                "elf_version": staged[4], "manifest_version": staged[5]}
    refused = re.search(r"(?m)^job=(\d+) complete kind=(\d+) status=([1-9]\d*)$", text)
    reason = re.search(r"(?m)^error: stage refused: (.+)$", text)
    if refused and reason and int(refused[2]) == STAGE_KIND:
        return {"job": int(refused[1]), "state": "refused", "status": int(refused[3]),
                "reason": reason[1].strip()}
    raise ValueError(f"unrecognized stage job output: {text!r}")


def _programs(rows):
    """Process identity only: resident service states change between listings."""
    return {pid: row["program"] for pid, row in rows.items()}


def _processes(uart):
    rows = {}
    for row in PROCESS_ROW.finditer(uart.command("ps")):
        rows[int(row[1])] = {"state": row[2], "program": row[7]}
    return rows


def _request_stage(uart, workspace, elf, manifest, elf_version, manifest_version):
    requested = uart.command(
        f"stage-ref {workspace} {elf} {manifest} {elf_version} {manifest_version}",
        "stage requested job=",
    )
    return int(re.search(r"stage requested job=(\d+)", requested)[1])


def _poll_stage(uart, job, done, delay=0.25):
    started = time.monotonic()
    while True:
        uart.send(f"job-status {job}\r".encode("ascii"))
        outcome = stage_outcome(uart.until())
        if outcome["job"] != job:
            raise AssertionError("job-status answered another job")
        if done(outcome):
            outcome["host_seconds"] = round(time.monotonic() - started, 3)
            return outcome
        if outcome["state"] != "pending":
            raise AssertionError(f"stage job ended early: {outcome}")
        if time.monotonic() - started > STAGE_TIMEOUT:
            raise AssertionError("stage job did not reach the expected state")
        time.sleep(delay)


def _stage(uart, *pair):
    return _poll_stage(uart, _request_stage(uart, *pair), lambda outcome: outcome["state"] != "pending")


def _cancel_case(uart, references, seeded):
    """Restart the file service while a stage has its kernel transaction open.

    The job must end superseded without a process. The stage that follows the
    restart in the caller then proves the kernel transaction was aborted: a
    leftover transaction would refuse the next STAGE_BEGIN as busy.
    """
    workspace = references["elf"]["workspace"]
    pair = (workspace, references["elf"]["resource"], references["manifest"]["resource"],
            version_text(seeded["elf"]["version"]), version_text(seeded["manifest"]["version"]))
    baseline = _programs(_processes(uart))
    job = _request_stage(uart, *pair)
    copying = _poll_stage(
        uart, job, lambda outcome: outcome["state"] == "pending" and outcome["phase"] == COPYING_PHASE,
        delay=0.05,
    )
    uart.command("restart files", "utility sessions revoked")
    uart.send(f"job-status {job}\r".encode("ascii"))
    outcome = stage_outcome(uart.until())
    if outcome["state"] != "refused" or outcome["status"] != SUPERSEDED:
        raise AssertionError(f"cancelled stage did not report superseded: {outcome}")
    # The restart gives the file service a new PID; compare the programs only.
    after = _programs(_processes(uart))
    if sorted(after.values()) != sorted(baseline.values()) or "staged" in after.values():
        raise AssertionError("cancelled stage left a process behind")
    return {"job": job, "restarted_after_seconds": copying["host_seconds"],
            "status": outcome["status"], "reason": outcome["reason"], "leftover_processes": 0}


def _stage_cases(uart, references, seeded, elf_size):
    """Refuse stale pins without a child, then stage, inspect, kill and reap one dormant child."""
    workspace = references["elf"]["workspace"]
    elf, manifest = references["elf"]["resource"], references["manifest"]["resource"]
    elf_version = version_text(seeded["elf"]["version"])
    manifest_version = version_text(seeded["manifest"]["version"])
    baseline = _programs(_processes(uart))
    if "staged" in baseline.values():
        raise AssertionError("a staged child exists before staging")
    refusals = []
    for name, pins in (
        ("stale_elf_version", (version_text(seeded["elf"]["version"] + 1), manifest_version)),
        ("stale_manifest_version", (elf_version, version_text(seeded["manifest"]["version"] + 1))),
    ):
        outcome = _stage(uart, workspace, elf, manifest, *pins)
        if outcome["state"] != "refused" or outcome["status"] != FILE_ERROR_BASE + FILE_VERSION:
            raise AssertionError(f"{name} was not refused as a version conflict: {outcome}")
        if _programs(_processes(uart)) != baseline:
            raise AssertionError(f"{name} refusal left a process behind")
        refusals.append({"case": name, "status": outcome["status"], "reason": outcome["reason"]})

    staged = _stage(uart, workspace, elf, manifest, elf_version, manifest_version)
    if staged["state"] != "staged":
        raise AssertionError(f"current pair was not staged: {staged}")
    if (staged["elf_version"], staged["manifest_version"]) != (elf_version, manifest_version):
        raise AssertionError("stage result does not echo the pinned versions")
    pid = staged["pid"]
    row = _processes(uart).get(pid)
    if row != {"state": "dormant", "program": "staged"}:
        raise AssertionError(f"staged child is not a dormant dynamic image: {row}")
    facts = re.search(
        r"scope=0 rights=0 generation=(\d+) expires=0 report=(\d+) bytes=(\d+) other=(\d+)",
        uart.command(f"permissions {pid}"),
    )
    ranges = (elf_size + CHUNK_BYTES - 1) // CHUNK_BYTES
    if not facts or int(facts[3]) != elf_size or int(facts[4]) != ranges or int(facts[1]) == 0:
        raise AssertionError("staged child facts do not match the provisioned ELF")
    uart.command(
        f"stage-ref {workspace} {elf} {manifest} {elf_version} {manifest_version}",
        "error: service busy or full",
    )
    uart.command(f"kill {pid}", "ok exit_kind=0 code=0")
    uart.command(f"reap {pid}", "ok exit_kind=3 code=0")
    if _programs(_processes(uart)) != baseline:
        raise AssertionError("reaping the staged child did not restore the process table")
    uart.command(f"reap {pid}", "error: service denied")
    return {
        "refusals": refusals,
        "staged": {
            "pid": pid,
            "state": row["state"],
            "program": row["program"],
            "elf_version": elf_version,
            "manifest_version": manifest_version,
            "kernel_generation": int(facts[1]),
            "guest_ticks": int(facts[2]),
            "bytes": int(facts[3]),
            "ranges": int(facts[4]),
            "host_seconds": staged["host_seconds"],
        },
        "second_stage_while_staged": "busy",
        "killed_and_reaped": True,
    }


def _volume_json(binary, *arguments):
    result = subprocess.run(
        [str(binary), *map(str, arguments)], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise RuntimeError(f"rustic-volume {arguments[0]} failed: {result.stderr.strip()}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"rustic-volume {arguments[0]} returned invalid JSON") from error


def _read_file(uart, refs, source):
    expected = source.read_bytes()
    offset = 0
    version = None
    observed = bytearray()
    while offset < len(expected):
        response = read_range(uart, refs, offset, min(CHUNK_BYTES, len(expected) - offset))
        result = response["result"]
        payload = base64.b64decode(result["data"])
        if result["size"] != len(expected) or result["offset"] != offset:
            raise AssertionError("guest read metadata does not match the provisioned artifact")
        if version is None:
            version = result["version"]
        elif version != result["version"]:
            raise AssertionError("artifact version changed during the bounded read")
        observed.extend(payload)
        offset += len(payload)
    if bytes(observed) != expected:
        raise AssertionError(f"guest bytes differ from {source.name}")
    return {
        "name": source.name,
        "size": len(observed),
        "sha256": environment.digest(source),
        "version": version,
        "ranges": (len(observed) + CHUNK_BYTES - 1) // CHUNK_BYTES,
    }


def _sample_file(uart, refs, source, offsets):
    expected = source.read_bytes()
    samples = []
    for offset in offsets:
        offset = min(offset, max(0, len(expected) - 1))
        result = read_range(uart, refs, offset, min(CHUNK_BYTES, len(expected) - offset))["result"]
        payload = base64.b64decode(result["data"])
        if payload != expected[offset : offset + len(payload)]:
            raise AssertionError(f"post-restart bytes differ from {source.name} at offset {offset}")
        samples.append({"offset": offset, "length": len(payload)})
    return samples


def _verify_report(report, seeded, elf, manifest):
    if report.get("lineage") != seeded["lineage"]:
        raise AssertionError("host remount changed the V7 lineage")
    nodes = {node["name"]: node for node in report.get("nodes", [])}
    for name, item, expected in (
        (elf.name, seeded["elf"], elf.stat().st_size),
        (manifest.name, seeded["manifest"], manifest.stat().st_size),
    ):
        node = nodes.get(name)
        if not node or node["id"] != item["id"] or node["size"] != expected or node["kind"] != "file":
            raise AssertionError(f"V7 host remount does not contain the expected {name}")
    workspace = nodes.get("application")
    if not workspace or workspace["id"] != seeded["workspace"]["id"] or workspace["kind"] != "directory":
        raise AssertionError("V7 host remount does not contain the expected application workspace")


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    if environment.digest(elf) != metadata["native_applications"]["file-server"][".elf"]:
        raise RuntimeError("file-server ELF differs from the artifact recorded by the boot build")
    if environment.digest(manifest) != metadata["native_applications"]["file-server"][".manifest"]:
        raise RuntimeError("file-server manifest differs from the artifact recorded by the boot build")

    reads, stages, serials, logs = [], [], [], []
    started = time.monotonic()
    (output / "result.json").unlink(missing_ok=True)
    (output / "terminal-v7.json").unlink(missing_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-volume.raw"
            lineage = uuid.uuid4().hex
            seeded = _volume_json(volume_tool, "seed7", data, lineage, elf, manifest)
            if seeded["lineage"] != lineage:
                raise AssertionError("host provisioner returned another lineage")
            initial_report = _volume_json(volume_tool, "report7", data)
            _verify_report(initial_report, seeded, elf, manifest)
            volume_bytes = data.stat().st_size
            before_sha256 = environment.digest(data)
            references = {
                "elf": {"workspace": seeded["workspace"]["text"], "resource": seeded["elf"]["resource"]},
                "manifest": {
                    "workspace": seeded["workspace"]["text"],
                    "resource": seeded["manifest"]["resource"],
                },
            }

            for phase in (1, 2):
                sock = temporary / f"uart-{phase}.sock"
                transcript = output / f"serial-{phase}.log"
                log = output / f"qemu-{phase}.log"
                serials.append(transcript)
                logs.append(log)
                with machine(
                    image,
                    data,
                    f"unix:{sock},server=on,wait=off",
                    log,
                ) as vm:
                    uart = Connection(sock, vm, transcript, BOOT_TIMEOUT, output / f"commands-{phase}.jsonl")
                    try:
                        uart.until()
                        if phase == 1:
                            uart.send(b"job-status 1\r")
                            startup_job = uart.until()
                            if "status=0" not in startup_job:
                                raise AssertionError(f"initial V7 mount job did not succeed: {startup_job!r}")
                        full_reads = [
                            _read_file(uart, references["elf"], elf),
                            _read_file(uart, references["manifest"], manifest),
                        ]
                        reads.append({"boot": phase, "full": full_reads})
                        if phase == 1:
                            stage = _stage_cases(uart, references, seeded, elf.stat().st_size)
                            stages.append({"boot": phase, "service_restart": False, **stage})
                            # This case performs the service restart itself.
                            cancelled = _cancel_case(uart, references, seeded)
                            offsets = [0, elf.stat().st_size // 2, elf.stat().st_size - CHUNK_BYTES]
                            samples = _sample_file(uart, references["elf"], elf, offsets)
                            for sample, offset in zip(samples, offsets):
                                expected_length = min(CHUNK_BYTES, elf.stat().st_size - offset)
                                if sample != {"offset": offset, "length": expected_length}:
                                    raise AssertionError("post-restart range read returned the wrong extent")
                            reads.append({"after_service_restart": samples})
                            stage = _stage_cases(uart, references, seeded, elf.stat().st_size)
                            stages.append({"boot": phase, "service_restart": True,
                                           "after_cancelled_stage": cancelled, **stage})
                        uart.send(b"exit\r")
                        uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
                        if vm.wait(timeout=10) != 33:
                            raise RuntimeError("unclean terminal-v7 exit")
                    finally:
                        uart.close()

            after_sha256 = environment.digest(data)
            final_report = _volume_json(volume_tool, "report7", data)
            _verify_report(final_report, seeded, elf, manifest)
            if before_sha256 != after_sha256 or initial_report != final_report:
                raise AssertionError("read-only guest boots changed the V7 volume")

        evidence = {
            "verified": True,
            "mode": "terminal-v7",
            "boots": 2,
            "service_restarts": 1,
            "data_drive_sha256_unchanged": True,
            "lineage": lineage,
            "artifact": {
                "elf": _read_file_summary(elf, metadata["native_applications"]["file-server"][".elf"]),
                "manifest": _read_file_summary(manifest, metadata["native_applications"]["file-server"][".manifest"]),
            },
            "reads": reads,
            "stages": stages,
            "volume": {
                "bytes": volume_bytes,
                "sha256_before": before_sha256,
                "sha256_after": after_sha256,
                "report": final_report,
            },
        }
        (output / "terminal-v7.json").write_text(json.dumps(evidence, separators=(",", ":")) + "\n")
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "terminal_v7": evidence,
        }
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print("V7 read acceptance: full ELF and manifest read over UART in two boots; service restart verified.", flush=True)
        for stage in stages:
            if "after_cancelled_stage" in stage:
                cancelled = stage["after_cancelled_stage"]
                print(f"V7 stage cancel: restart during copy -> status={cancelled['status']} "
                      f"({cancelled['reason']}), no leftover process; the next stage succeeded.", flush=True)
            staged = stage["staged"]
            print(
                f"V7 stage: pid={staged['pid']} dormant bytes={staged['bytes']} ranges={staged['ranges']} "
                f"guest_ticks={staged['guest_ticks']} host_seconds={staged['host_seconds']} "
                f"after_restart={stage['service_restart']}; stale ELF/manifest pins refused without a child.",
                flush=True,
            )
        return result
    finally:
        (output / "serial.log").write_bytes(b"\n".join(path.read_bytes() for path in serials if path.exists()))
        (output / "qemu.log").write_bytes(b"\n".join(path.read_bytes() for path in logs if path.exists()))


def _read_file_summary(path, expected_sha256):
    size = path.stat().st_size
    actual_sha256 = environment.digest(path)
    if actual_sha256 != expected_sha256:
        raise AssertionError(f"{path.name} changed after the boot image was built")
    return {"name": path.name, "size": size, "sha256": actual_sha256}
