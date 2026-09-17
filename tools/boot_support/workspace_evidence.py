# SPDX-License-Identifier: Apache-2.0
"""A host-provisioned v6 workspace on the disposable disk, read by the guest and
checked by the independent reader (#51).

The volume is written by `rustic-volume` (the same writer the tests use) and
placed outside both v5 volumes. The guest mounts it, reads a 16 KiB artifact in
4 KiB ranges, publishes a tracked write whose receipt the second boot must find
and replay, and leaves what it saw in the sector before the volume. This module
recomputes the digest from the artifact, re-reads the volume with
`terminal_support.oracle6` and requires the same volume digest across the two
boots, so agreement is between the guest, the writer and an independent reader
rather than with the code under test.
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


def inspect(disk, output, phase):
    """Verify the volume and the observation the guest left for this boot."""
    with disk.open("rb") as source:
        source.seek(OFFSET)
        region = source.read(PREFIX)
        source.seek(EVIDENCE * 512)
        record = source.read(512)
    evidence = Path(output) / "workspace.bin"
    evidence.write_bytes(region)
    state = oracle6.snapshot(region)
    artifact = state["nodes"].get(5)
    if artifact is None or artifact["length"] != LENGTH or artifact["name"] != "artifact" or not artifact["extents"]:
        raise RuntimeError("the guest-visible workspace volume is not the fixture artifact")
    if artifact["length"] <= 1024:
        raise RuntimeError("the workspace artifact is not beyond the v5 file limit")
    if state["files"].get((4, "artifact")) != PATTERN:
        raise RuntimeError("independent read of the workspace artifact differs")
    expected_digest = digest()

    # The tracked write the guest published: one new file whose bytes are the
    # artifact digest, named by one retained receipt.
    guest = [node for node in state["nodes"].values() if node["name"] == "guest"]
    if len(guest) != 1 or guest[0]["kind"] != "file":
        raise RuntimeError("the guest did not publish exactly one tracked file")
    guest = guest[0]
    if len(state["receipts"]) != 1:
        raise RuntimeError("the tracked write retained no receipt")
    receipt = state["receipts"][0]
    if (receipt["id"], receipt["previous"], receipt["length"]) != (guest["id"], 1, 8):
        raise RuntimeError("the retained receipt does not name the guest file")
    if guest["version"] != receipt["committed"]:
        raise RuntimeError("the receipt and the guest file version disagree")
    if state["files"].get((4, "guest")) != expected_digest.to_bytes(8, "little"):
        raise RuntimeError("the tracked payload is not the artifact digest")

    if record[:8] != b"RUSTICW1":
        raise RuntimeError("the guest left no workspace observation")
    length, observed, ranges = (
        int.from_bytes(record[8:16], "little"),
        int.from_bytes(record[16:24], "little"),
        int.from_bytes(record[24:28], "little"),
    )
    if (length, ranges) != (LENGTH, RANGES) or observed != expected_digest:
        raise RuntimeError("the guest read a different artifact or range count")
    reported = {
        "id": int.from_bytes(record[32:36], "little"),
        "committed": int.from_bytes(record[36:44], "little"),
        "length": int.from_bytes(record[44:48], "little"),
    }
    if (reported["id"], reported["committed"], reported["length"]) != (
            receipt["id"], receipt["committed"], receipt["length"]):
        raise RuntimeError("the guest reported a different receipt than the volume holds")
    if record[28] != phase or record[48] != phase:
        raise RuntimeError(f"the guest observation does not belong to boot {phase}")
    return {"phase": "write" if phase == 1 else "replay",
            "volume_sha256": hashlib.sha256(region).hexdigest(),
            "artifact_sha256": hashlib.sha256(PATTERN).hexdigest(),
            "digest": f"{observed:016x}", "length": length, "ranges": ranges,
            "receipt": {"id": receipt["id"], "previous": receipt["previous"],
                        "committed": receipt["committed"], "length": receipt["length"]},
            "guest_verified": True}
