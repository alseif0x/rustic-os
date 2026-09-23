# SPDX-License-Identifier: Apache-2.0
"""Read a provisioned V7 application artifact in a disposable UEFI guest."""
import base64
import json
from pathlib import Path
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

    reads, serials, logs = [], [], []
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
                            uart.command("restart files", "utility sessions revoked")
                            offsets = [0, elf.stat().st_size // 2, elf.stat().st_size - CHUNK_BYTES]
                            samples = _sample_file(uart, references["elf"], elf, offsets)
                            for sample, offset in zip(samples, offsets):
                                expected_length = min(CHUNK_BYTES, elf.stat().st_size - offset)
                                if sample != {"offset": offset, "length": expected_length}:
                                    raise AssertionError("post-restart range read returned the wrong extent")
                            reads.append({"after_service_restart": samples})
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
