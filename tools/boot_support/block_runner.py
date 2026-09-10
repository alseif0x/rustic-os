# SPDX-License-Identifier: Apache-2.0
"""Own a fresh sparse disk across separate QEMU processes; verify guest writes on host."""
import hashlib
import json
from pathlib import Path
import tempfile

SIZE = 4 * 1024 ** 3
SECTORS = (0, 8, 9, SIZE // 512 - 1)


def pattern(sector):
    return bytes((sector * 17 + i * 29) % 251 + 1 for i in range(512))


def inspect_disk(disk, written):
    expected = b"".join(pattern(sector) if written and sector else bytes(512) for sector in SECTORS)
    with disk.open("rb") as source:
        selected = b""
        for sector in SECTORS:
            source.seek(sector * 512)
            selected += source.read(512)
    if disk.stat().st_size != SIZE or selected != expected:
        raise RuntimeError("persistent disk content differs from independent host oracle")
    return selected


def run(image, timeout, run_once):
    from .scenarios import records
    from .block_evidence import verified
    image = Path(image).resolve()
    directory = image.parent
    mode = json.loads((directory / "image.json").read_text())["mode"]
    phases = []
    serials, logs = [], []
    with tempfile.TemporaryDirectory(prefix="rustic-block-") as temporary:
        disk = Path(temporary) / "disposable.raw"
        with disk.open("xb") as output:
            output.truncate(SIZE)  # Sparse logical size; never open a caller-supplied disk.
        selected = inspect_disk(disk, False)
        arguments = []
        if mode != "block-missing":
            readonly = ",readonly=on" if mode == "block-readonly" else ""
            arguments = ["-drive", f"if=none,id=rusticdata,format=raw,cache=writeback,file={disk}{readonly}",
                         "-device", "virtio-blk-pci,drive=rusticdata,addr=0x6,disable-modern=on,disable-legacy=off,queue-size=8,num-queues=1,vectors=0,rerror=report,werror=report"]
        for number in range(2 if mode in ("block-persist", "block-user") else 1):
            output = directory / f"phase-{number + 1}"
            output.mkdir(exist_ok=True)
            result = run_once(image, timeout, arguments, output)
            serials.append((output / "serial.log").read_text())
            logs.append((output / "qemu.log").read_text())
            phases.append(result)
            if result["outcome"] != "success":
                break
            selected = inspect_disk(disk, mode in ("block-persist", "block-user"))
        allocation = disk.stat().st_blocks * 512
    serial = "\n".join(serials)
    (directory / "serial.log").write_text(serial)
    (directory / "qemu.log").write_text("\n".join(logs))
    evidence = {"logical_bytes": SIZE, "allocated_bytes": allocation,
                "selected_sectors": SECTORS, "selected_sha256": hashlib.sha256(selected).hexdigest(),
                "separate_vm_boots": len(phases), "host_verified": all(p["outcome"] == "success" for p in phases)}
    (directory / "blocks.bin").write_bytes(selected)
    (directory / "block.json").write_text(json.dumps(evidence, indent=2) + "\n")
    result = {**phases[-1], "phases": phases, "block": evidence,
              "elapsed_seconds": round(sum(p["elapsed_seconds"] for p in phases), 3)}
    if any(p["outcome"] != "success" for p in phases) or not verified(mode, serial, records):
        result["outcome"] = "unexpected"
    (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result
