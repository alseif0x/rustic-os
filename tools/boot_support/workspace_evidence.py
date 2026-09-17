# SPDX-License-Identifier: Apache-2.0
"""A host-provisioned v6 workspace on the disposable disk, read by the guest and
checked by the independent reader (#51).

The volume is written by `rustic-volume` (the same writer the tests use) and
placed outside both v5 volumes. The guest mounts it, reads a 16 KiB artifact in
4 KiB ranges and leaves a digest in the sector before the volume; this module
recomputes that digest from the artifact and re-reads the volume with
`terminal_support.oracle6`, so agreement is between the guest, the writer and an
independent reader rather than with the code under test.
"""
import hashlib
import subprocess
import tempfile
from pathlib import Path

from terminal_support import oracle6

BASE = 1024
OFFSET = BASE * 512
EVIDENCE = BASE - 1
LENGTH = 16_384
RANGES = (LENGTH + 4095) // 4096
LINEAGE = "05" * 16
PATTERN = bytes(index % 251 for index in range(LENGTH))
# Structures plus the first payload sectors the fixture artifact occupies.
PREFIX = (oracle6.PAYLOAD_SECTOR + 512) * 512
ROOT = Path(__file__).resolve().parents[2]


def digest():
    value = 0xCBF29CE484222325
    for byte in PATTERN:
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value


def tool(*arguments):
    binary = ROOT / "target/debug/rustic-volume"
    if not binary.is_file():
        built = subprocess.run(["cargo", "build", "-p", "rustic-volume", "--locked"],
                               cwd=ROOT, capture_output=True, text=True)
        if built.returncode != 0:
            raise RuntimeError("cannot build rustic-volume: " + built.stdout + built.stderr)
    result = subprocess.run([binary, *arguments], cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"rustic-volume {arguments[0]} failed: {result.stdout}{result.stderr}")
    return result.stdout


def provision(disk):
    with tempfile.TemporaryDirectory(prefix="rustic-workspace-") as temporary:
        directory = Path(temporary)
        image, artifact = directory / "workspace.img", directory / "artifact.bin"
        artifact.write_bytes(PATTERN)
        tool("provision", str(image), LINEAGE)
        tool("write", str(image), "4", "artifact", str(artifact))
        data = image.read_bytes()
    if len(data) < PREFIX:
        raise RuntimeError("the provisioned workspace image is shorter than its structures")
    with disk.open("r+b") as target:
        target.seek(OFFSET)
        target.write(data[:PREFIX])


def inspect(disk, output):
    with disk.open("rb") as source:
        source.seek(OFFSET)
        region = source.read(PREFIX)
        source.seek(EVIDENCE * 512)
        record = source.read(512)
    evidence = Path(output) / "workspace.bin"
    evidence.write_bytes(region)
    state = oracle6.snapshot(region)
    files = state["files"]
    node = state["nodes"].get(5)
    if node is None or node["length"] != LENGTH or node["name"] != "artifact" or not node["extents"]:
        raise RuntimeError("the guest-visible workspace volume is not the fixture artifact")
    if node["length"] <= 1024:
        raise RuntimeError("the workspace artifact is not beyond the v5 file limit")
    if files.get((4, "artifact")) != PATTERN:
        raise RuntimeError("independent read of the workspace artifact differs")
    if record[:8] != b"RUSTICW1":
        raise RuntimeError("the guest left no workspace observation")
    length, observed, ranges = (
        int.from_bytes(record[8:16], "little"),
        int.from_bytes(record[16:24], "little"),
        int.from_bytes(record[24:28], "little"),
    )
    if (length, ranges) != (LENGTH, RANGES):
        raise RuntimeError("the guest read a different length or range count")
    if observed != digest():
        raise RuntimeError("the guest digest does not match the host artifact")
    return {"file_sha256": hashlib.sha256(PATTERN).hexdigest(), "digest": f"{observed:016x}",
            "length": length, "ranges": ranges, "extents": len(node["extents"]),
            "guest_verified": True}
