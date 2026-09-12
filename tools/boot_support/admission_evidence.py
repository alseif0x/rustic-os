# SPDX-License-Identifier: Apache-2.0
"""Native admission fixtures occupy two independent disposable volume regions."""
from pathlib import Path
import tempfile
from terminal_support.provision import provision as provision_volume
from terminal_support.oracle import snapshot

BASES = (512, 768)
SIZE = 174 * 512


def provision(disk):
    for base in BASES:
        with tempfile.TemporaryFile() as volume:
            volume.truncate(SIZE)
            provision_volume(volume.fileno())
            volume.seek(0)
            with disk.open("r+b") as target:
                target.seek(base * 512)
                target.write(volume.read())


def inspect(disk, output):
    results = []
    for base, name, content, states in [(512, "terminal", b"after", [(51, "cancelled", b"cancelled bytes"), (52, "committed", b"after")]),
                                         (768, "pending", b"before", [(53, "admitted", b"unexecuted")])]:
        with disk.open("rb") as source:
            source.seek(base * 512)
            region = source.read(SIZE + 512)
        evidence = Path(output) / f"admission-{name}.bin"
        evidence.write_bytes(region)
        _, state = snapshot(evidence)
        if state["format"] != (5 if name == "terminal" else 4) or state["files"] != {(4, "admission"): content}:
            raise RuntimeError("native admission file differs from host oracle")
        observed = [(r["key"], r.get("state"), r["content"]) for r in state["records"]]
        if observed != states:
            raise RuntimeError("native admission states differ from host oracle")
        for r in state["records"]:
            if (r["subject"], r["workspace"], r["epoch"]) != (9, 4, 1):
                raise RuntimeError("native admission namespace differs from host oracle")
            if r["state"] == "committed" and state["nodes"][r["id"]]["version"] != r["committed"]:
                raise RuntimeError("admission receipt differs from committed file version")
            if name == "pending" and state["nodes"][r["id"]]["version"] != r["previous"]:
                raise RuntimeError("unexecuted admission changed file version")
            if name == "terminal" and r.get("prevention") != ("authority_lost" if r["key"] == 51 else None):
                raise RuntimeError("native prevention cause differs from host oracle")
        results.append({"volume": name, "selected_sha256": state["selected_sha256"], "sequence": state["sequence"],
                        "states": [r["state"] for r in state["records"]], "host_verified": True,
                        "format": state["format"], "prevention": [r.get("prevention") for r in state["records"]]})
    return results
