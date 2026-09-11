# SPDX-License-Identifier: Apache-2.0
"""Disjoint native publication fixture, checked by the independent volume reader."""
from pathlib import Path
import tempfile
from terminal_support.provision import provision as provision_volume
from terminal_support.oracle import snapshot

OFFSET = 256 * 512
SIZE = 174 * 512


def provision(disk):
    with tempfile.TemporaryFile() as volume:
        volume.truncate(SIZE)
        provision_volume(volume.fileno())
        volume.seek(0)
        with disk.open("r+b") as target:
            target.seek(OFFSET)
            target.write(volume.read())


def inspect(disk, output):
    with disk.open("rb") as source:
        source.seek(OFFSET)
        region = source.read(SIZE + 512)  # Include the untouched trailing guard.
    evidence = Path(output) / "publication.bin"
    evidence.write_bytes(region)
    _, state = snapshot(evidence)
    if state["format"] != 3 or state["files"] != {(4, "publication"): b"after"}:
        raise RuntimeError("native publication file differs from host oracle")
    if len(state["records"]) != 1:
        raise RuntimeError("cancelled preparations published unexpected receipts")
    record = state["records"][0]
    if (record["subject"], record.get("workspace"), record["epoch"], record["key"], record["content"]) != (9, 4, 1, 42, b"after"):
        raise RuntimeError("native publication receipt differs from host oracle")
    if record["committed"] != state["sequence"] or state["nodes"][record["id"]]["version"] != record["committed"]:
        raise RuntimeError("receipt and file publication versions differ")
    return {"selected_sha256": state["selected_sha256"], "sequence": state["sequence"],
            "records": 1, "bytes": 5, "host_verified": True}
