# SPDX-License-Identifier: Apache-2.0
"""Disjoint V7 publication fixture checked by the independent oracle7 reader."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import uuid

from terminal_support import oracle7

OFFSET = 256
SIZE = oracle7.VOLUME_SECTORS
GUARD = OFFSET + SIZE
ROOT = Path(__file__).resolve().parents[2]


def provision(disk):
    with tempfile.TemporaryDirectory(prefix="rustic-publication-v7-") as temporary:
        volume = Path(temporary) / "volume.raw"
        tool = Path(os.environ.get("RUSTIC_VOLUME_TOOL", ROOT / "target/debug/rustic-volume"))
        subprocess.run([str(tool), "provision7", str(volume), uuid.uuid4().hex],
                       check=True, stdout=subprocess.DEVNULL)
        with volume.open("rb") as source, disk.open("r+b") as target:
            target.seek(OFFSET * 512)
            target.write(source.read())


def inspect(disk, output):
    with disk.open("rb") as source:
        source.seek(OFFSET * 512)
        region = source.read(SIZE * 512)
        guard = source.read(512)
    if len(region) != SIZE * 512 or guard != bytes(512):
        raise RuntimeError("V7 publication region or trailing guard was truncated or changed")
    evidence = Path(output) / "publication.bin"
    evidence.write_bytes(region)
    state = oracle7.snapshot(region)
    expected = {"/config/owner-policy": b"rustic-owner-v1\nhelpers=explicit\n",
                "/workspaces/publication": b"after"}
    if state["format"] != 7 or state["contents"] != expected:
        raise RuntimeError("native V7 publication files differ from host oracle")
    if len(state["records"]) != 1:
        raise RuntimeError("cancelled V7 preparations published unexpected receipts")
    record = state["records"][0]
    identity = (record["subject"], record["workspace"], record["epoch"], record["key"])
    if identity != (9, 4, 1, 42) or record["state"] != "direct_committed":
        raise RuntimeError("native V7 publication receipt differs from host oracle")
    if (record["length"], record["sha256"]) != (5, hashlib.sha256(b"after").hexdigest()):
        raise RuntimeError("native V7 publication snapshot differs from host oracle")
    node = state["nodes"][record["object"]]
    if record["committed"] != state["sequence"] or node["version"] != record["committed"]:
        raise RuntimeError("V7 receipt and file publication versions differ")
    return {"selected_sha256": hashlib.sha256(region).hexdigest(), "sequence": state["sequence"],
            "records": 1, "bytes": 5, "format": 7, "host_verified": True}
