# SPDX-License-Identifier: Apache-2.0
"""Boot `terminal-v7` on deliberately damaged disposable V7 volumes (#51).

Two copies of one freshly seeded application volume are damaged with the
independent reader (`oracle7`) locating the bytes, and each is checked on the
host by both `oracle7` and `report7` before the guest sees it:

- `payload`: one byte inside the ELF payload is flipped. Mount must refuse the
  volume through the live payload CRC. The guest must not panic; the file
  service exits with its documented startup status for `Corrupt`, reads report
  `Unavailable`, a service restart is refused the same way, and owner control
  (`ps`, `mem`, `services`, `exit`) stays usable.
- `header`: the newest header copy's checksum is broken while the older copy is
  valid. Mount must select the older complete generation, so the guest reads the
  ELF byte-for-byte and sees the manifest at its older, empty version, while the
  newer manifest version is a version conflict.

Neither case writes the volume: the SHA-256 is compared before and after boot.
The host tool's `recovered` flag is not observable in the guest; the older
generation is identified by the versions and sizes the guest reads.
"""
import base64
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile
import time
import uuid

import environment
from . import oracle7
from .connection import Connection
from .machine import machine
from .read_cases import read as read_range
from .v7_read import version_text, volume_json


ROOT = environment.ROOT
BOOT_TIMEOUT = 300
CHUNK_BYTES = 1024
# `startup_error` in apps/file-server/src/main.rs: rustic_fs::Error::Corrupt.
FILE_STARTUP_CORRUPT = 5
# Kernel exit kind for an explicit process exit code (`exit_words`).
EXIT_CODE = 1
# The supervisor's mount job reports any readiness mismatch as status 4.
MOUNT_JOB_KIND = 11
MOUNT_REFUSED = 4
PROCESS_ROW = re.compile(r"(?m)^(\d+) (\w+) (\d+) (\d+) (\d+) (\d+) (\S+)\r?$")
MOUNT_JOB = re.compile(r"(?m)^job=(\d+) complete kind=(\d+) status=(\d+) ")
MEM_ROW = re.compile(r"(?m)^ticks=\d+ free_frames=\d+ process_slots=\d+ processes=\d+ ")


def processes(text):
    return {int(row[1]): {"state": row[2], "exit": int(row[3]), "code": int(row[4]), "program": row[7]}
            for row in PROCESS_ROW.finditer(text)}


def files_row(uart):
    rows = [row for row in processes(uart.command("ps")).values() if row["program"] == "files"]
    if len(rows) != 1:
        raise AssertionError(f"expected one files process row, saw {rows}")
    return rows[0]


def mount_job(uart, job):
    uart.send(f"job-status {job}\r".encode("ascii"))
    text = uart.until()
    match = MOUNT_JOB.search(text)
    if not match or int(match[1]) != job or int(match[2]) != MOUNT_JOB_KIND:
        raise AssertionError(f"unrecognized mount job status: {text!r}")
    return int(match[3]), text


def owner_control(uart):
    """Owner-side commands that must keep working whatever storage did."""
    rows = processes(uart.command("ps"))
    if {"supervisor", "shell"} - {row["program"] for row in rows.values()}:
        raise AssertionError("supervisor or shell is missing from the process table")
    if not MEM_ROW.search(uart.command("mem")):
        raise AssertionError("mem did not report kernel counters")
    services = uart.command("services")
    if "shell pid=" not in services:
        raise AssertionError("services did not report the shell")
    return {"ps": sorted(row["program"] for row in rows.values()), "mem": True, "services": True}


def flip(path, offset):
    with path.open("r+b") as handle:
        handle.seek(offset)
        value = handle.read(1)[0]
        handle.seek(offset)
        handle.write(bytes([value ^ 0x01]))


def host_view(tool, path):
    """Both host readers on one image: (oracle state or refusal, report or refusal)."""
    try:
        state, oracle_refusal = oracle7.snapshot(path.read_bytes()), None
    except oracle7.Corrupt as error:
        state, oracle_refusal = None, str(error)
    try:
        report, rust_refusal = volume_json(tool, "report7", path), None
    except RuntimeError as error:
        report, rust_refusal = None, str(error)
    return state, oracle_refusal, report, rust_refusal


def read_exact(uart, refs, expected, version):
    """Read a whole file in bounded ranges; bytes and pinned version must match."""
    observed = bytearray()
    while len(observed) < len(expected):
        result = read_range(uart, refs, len(observed), min(CHUNK_BYTES, len(expected) - len(observed)),
                            version=version)["result"]
        if result["size"] != len(expected) or result["version"] != version:
            raise AssertionError("guest read metadata differs from the selected generation")
        observed.extend(base64.b64decode(result["data"]))
    if bytes(observed) != expected:
        raise AssertionError("guest bytes differ from the selected generation")
    return {"size": len(observed), "version": version, "sha256": hashlib.sha256(observed).hexdigest(),
            "ranges": (len(observed) + CHUNK_BYTES - 1) // CHUNK_BYTES}


def boot(image, data, output, name, body):
    sock_dir = Path(tempfile.mkdtemp(prefix="rustic-v7-corrupt-"))
    try:
        sock = sock_dir / "uart.sock"
        transcript = output / f"serial-{name}.log"
        with machine(image, data, f"unix:{sock},server=on,wait=off", output / f"qemu-{name}.log") as vm:
            uart = Connection(sock, vm, transcript, BOOT_TIMEOUT, output / f"commands-{name}.jsonl")
            try:
                banner = uart.until()
                observed = body(uart, banner)
                uart.send(b"exit\r")
                uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
                returncode = vm.wait(timeout=10)
            finally:
                uart.close()
        serial = transcript.read_bytes()
        for marker in (b"RUSTIC PANIC", b"RUSTIC FATAL", b"RUSTIC EXCEPTION", b"RUSTIC FAULT",
                       b"RUSTIC PROCESS_FAULT", b"RUSTIC MEMORY_FAULT"):
            if marker in serial:
                raise AssertionError(f"{name}: guest reported {marker.decode()}")
        if returncode != 33:
            raise AssertionError(f"{name}: unclean terminal-v7 exit {returncode}")
        return {**observed, "exit": returncode}
    finally:
        shutil.rmtree(sock_dir, ignore_errors=True)


def payload_case(uart, banner, refs):
    """Mount refused: no panic, files unavailable, owner control usable."""
    if "error: service unavailable" not in banner:
        raise AssertionError("the shell did not report the file service unavailable at startup")
    status, _ = mount_job(uart, 1)
    if status != MOUNT_REFUSED:
        raise AssertionError(f"mount job did not fail: status={status}")
    row = files_row(uart)
    if (row["state"], row["exit"], row["code"]) != ("exited", EXIT_CODE, FILE_STARTUP_CORRUPT):
        raise AssertionError(f"file service did not exit with the Corrupt startup status: {row}")
    control = owner_control(uart)
    reads = {}
    for name, ref in refs.items():
        reads[name] = read_range(uart, ref, 0, 16, expected_code="unavailable")["error"]["code"]
    uart.command("restart files", "error: service unavailable")
    restart_status, _ = mount_job(uart, 2)
    restarted = files_row(uart)
    if restart_status != MOUNT_REFUSED or (restarted["exit"], restarted["code"]) != (EXIT_CODE, FILE_STARTUP_CORRUPT):
        raise AssertionError("a service restart did not refuse the corrupt volume the same way")
    after = owner_control(uart)
    return {"startup": "service unavailable", "mount_job_status": status, "files_exit": row,
            "reads": reads, "restart_mount_job_status": restart_status, "restarted_files_exit": restarted,
            "owner_control_before_restart": control, "owner_control_after_restart": after}


def header_case(uart, banner, refs, expected, newer_manifest_version):
    """Mount recovered the older generation: exact bytes at its versions."""
    if "error:" in banner:
        raise AssertionError("the file service did not start on the recoverable volume")
    status, _ = mount_job(uart, 1)
    if status != 0:
        raise AssertionError(f"mount job failed on the recoverable volume: status={status}")
    if files_row(uart)["state"] == "exited":
        raise AssertionError("the file service exited on the recoverable volume")
    reads = {name: read_exact(uart, refs[name], content, version_text(version))
             for name, (version, content) in expected.items()}
    empty = read_range(uart, refs["manifest"], 0, 16, version=version_text(expected["manifest"][0]))["result"]
    if empty["size"] != 0 or empty["eof"] is not True or base64.b64decode(empty["data"]):
        raise AssertionError("the older manifest version is not empty in the guest")
    conflict = read_range(uart, refs["manifest"], 0, 16, version=version_text(newer_manifest_version),
                          expected_code="version_conflict")["error"]["code"]
    return {"mount_job_status": status, "reads": reads, "manifest_older_version_empty": True,
            "newer_manifest_version": conflict, "owner_control": owner_control(uart)}


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-corrupt")
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").unlink(missing_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    for path, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(path) != metadata["native_applications"]["file-server"][suffix]:
            raise RuntimeError(f"{path.name} differs from the artifact recorded by the boot build")
    started = time.monotonic()
    cases = {}
    with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-corrupt-") as temporary:
        temporary = Path(temporary)
        base = temporary / "base.raw"
        lineage = uuid.uuid4().hex
        seeded = volume_json(volume_tool, "seed7", base, lineage, elf, manifest)
        state = oracle7.snapshot(base.read_bytes())
        refs = {name: {"workspace": seeded["workspace"]["text"], "resource": seeded[name]["resource"]}
                for name in ("elf", "manifest")}
        elf_node = state["nodes"][seeded["elf"]["id"]]

        # (a) One flipped byte in the middle of the live ELF payload.
        damaged = temporary / "payload.raw"
        shutil.copyfile(base, damaged)
        middle = elf_node["length"] // 2
        run_start, run_sectors = elf_node["runs"][0]
        if middle // oracle7.SECTOR >= run_sectors:
            raise AssertionError("the ELF's first run does not hold its middle byte")
        flip(damaged, (oracle7.PAYLOAD_SECTOR + run_start) * oracle7.SECTOR + middle)
        view, oracle_refusal, _, rust_refusal = host_view(volume_tool, damaged)
        if view is not None or rust_refusal is None:
            raise AssertionError("a host reader accepted the flipped payload byte")
        before = environment.digest(damaged)
        guest = boot(image, damaged, output, "payload", lambda uart, banner: payload_case(uart, banner, refs))
        if environment.digest(damaged) != before:
            raise AssertionError("the guest changed the refused volume")
        cases["payload"] = {"damage": {"file": "file-server.elf", "offset": middle},
                            "host": {"oracle7": oracle_refusal, "report7": rust_refusal},
                            "guest": guest, "sha256_unchanged": True}

        # (b) Newest header copy torn, older copy valid.
        torn = temporary / "header.raw"
        shutil.copyfile(base, torn)
        newest = state["generation"]
        flip(torn, oracle7.header_sector(newest) * oracle7.SECTOR + 20)
        view, oracle_refusal, report, rust_refusal = host_view(volume_tool, torn)
        if view is None or report is None:
            raise AssertionError(f"a host reader refused the recoverable volume: {oracle_refusal or rust_refusal}")
        if (view["generation"], view["sequence"], view["recovered"]) != (1 - newest, state["sequence"] - 1, True) \
                or (report["generation"], report["sequence"], report["recovered"]) != \
                (view["generation"], view["sequence"], True):
            raise AssertionError("the host readers did not both select the older generation")
        by_name = {Path(entry["path"]).name: entry for entry in view["files"]}
        older = {"elf": (by_name[elf.name]["version"], view["contents"][by_name[elf.name]["path"]]),
                 "manifest": (by_name[manifest.name]["version"], view["contents"][by_name[manifest.name]["path"]])}
        if older["elf"] != (seeded["elf"]["version"], elf.read_bytes()) or older["manifest"][1] != b"" \
                or older["manifest"][0] >= seeded["manifest"]["version"]:
            raise AssertionError("the older generation is not the ELF-only generation")
        before = environment.digest(torn)
        guest = boot(image, torn, output, "header",
                     lambda uart, banner: header_case(uart, banner, refs, older, seeded["manifest"]["version"]))
        if environment.digest(torn) != before:
            raise AssertionError("the guest changed the recovered volume")
        cases["header"] = {"damage": {"header_slot": newest},
                           "host": {"oracle7": {key: view[key] for key in ("generation", "sequence", "recovered",
                                                                           "rejected_headers", "files")},
                                    "report7": {key: report[key] for key in ("generation", "sequence", "recovered")}},
                           "guest": guest, "sha256_unchanged": True}

    result = {"outcome": "success", "mode": "terminal-v7", "build_id": metadata["build_id"],
              "image_sha256": metadata["image_sha256"], "lineage": lineage,
              "elapsed_seconds": round(time.monotonic() - started, 3), "cases": cases}
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"V7 corrupt-input acceptance: flipped payload refused (files exit code {FILE_STARTUP_CORRUPT}, "
          f"mount job status {MOUNT_REFUSED}, reads Unavailable, owner control usable); torn newest header "
          f"recovered generation {cases['header']['host']['oracle7']['generation']} and read exact bytes.",
          flush=True)
    return result
