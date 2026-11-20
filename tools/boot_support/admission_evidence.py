# SPDX-License-Identifier: Apache-2.0
"""Native V7 admission fixtures occupy independent guarded volume regions."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import uuid

from terminal_support import oracle7

BASES = (131_539, 262_822)
SIZE = oracle7.VOLUME_SECTORS
ROOT = Path(__file__).resolve().parents[2]


def provision(disk):
    tool = Path(os.environ.get("RUSTIC_VOLUME_TOOL", ROOT / "target/debug/rustic-volume"))
    for base in BASES:
        with tempfile.TemporaryDirectory(prefix="rustic-admission-v7-") as temporary:
            volume = Path(temporary) / "volume.raw"
            subprocess.run([str(tool), "provision7", str(volume), uuid.uuid4().hex],
                           check=True, stdout=subprocess.DEVNULL)
            with volume.open("rb") as source, disk.open("r+b") as target:
                target.seek(base * 512)
                target.write(source.read())


def inspect(disk, output):
    results = []
    cases = [(BASES[0], "terminal", b"after",
              [(51, "cancelled", b"cancelled bytes"), (52, "admitted_committed", b"after")]),
             (BASES[1], "pending", b"before", [(53, "admitted", b"unexecuted")])]
    for base, name, content, states in cases:
        with disk.open("rb") as source:
            source.seek(base * 512)
            region = source.read(SIZE * 512)
            guard = source.read(512)
        if len(region) != SIZE * 512 or guard != bytes(512):
            raise RuntimeError("V7 admission region or trailing guard was truncated or changed")
        evidence = Path(output) / f"admission-{name}.bin"
        evidence.write_bytes(region)
        state = oracle7.snapshot(region)
        expected_contents = {"/config/owner-policy": b"rustic-owner-v1\nhelpers=explicit\n",
                             "/workspaces/admission": content}
        if state["format"] != 7 or state["contents"] != expected_contents:
            raise RuntimeError("native V7 admission files differ from host oracle")
        observed = [(r["key"], r["state"], r["length"], r["sha256"])
                    for r in sorted(state["records"], key=lambda record: record["key"])]
        expected = [(key, status, len(payload), hashlib.sha256(payload).hexdigest())
                    for key, status, payload in states]
        if observed != expected:
            raise RuntimeError("native V7 admission states differ from host oracle")
        for record in state["records"]:
            if (record["subject"], record["workspace"], record["epoch"]) != (9, 4, 1):
                raise RuntimeError("native V7 admission namespace differs from host oracle")
            node = state["nodes"][record["object"]]
            if record["state"] == "admitted_committed" and node["version"] != record["committed"]:
                raise RuntimeError("V7 admission receipt differs from committed file version")
            if name == "pending" and node["version"] != record["previous"]:
                raise RuntimeError("unexecuted V7 admission changed file version")
            expected_cause = "authority_lost" if record["key"] == 51 else None
            if record["cause"] != expected_cause:
                raise RuntimeError("native V7 prevention cause differs from host oracle")
        results.append({"volume": name, "selected_sha256": hashlib.sha256(region).hexdigest(),
                        "sequence": state["sequence"],
                        "states": [r["state"] for r in sorted(state["records"], key=lambda r: r["key"])],
                        "host_verified": True, "format": 7,
                        "prevention": [r["cause"] for r in sorted(state["records"], key=lambda r: r["key"])]})
    return results
