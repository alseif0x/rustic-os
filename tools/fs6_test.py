# SPDX-License-Identifier: Apache-2.0
"""Verify v6 volume images with an independent reader (#51).

`cargo test -p rustic-fs --test fs6_image` provisions and migrates real volume
images and exports the prefix each one uses. This suite reads those images with
`terminal_support.oracle6`, a reader written from the format description rather
than from the Rust code, and compares the migration output with what
`terminal_support.oracle` (the existing v5 reader) says about the source image.
It also records what the reader refuses, so a checksum that stops being checked
is visible.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path

import environment
from terminal_support import oracle, oracle6

DEFAULT_OUTPUT = environment.ROOT / "artifacts/fs6"
BIG = 200_000
PATTERN = bytes(index % 251 for index in range(BIG))


def export(output):
    """Run the Rust image test, which writes the images this suite verifies."""
    images = output / "images"
    shutil.rmtree(images, ignore_errors=True)
    images.mkdir(parents=True)
    command = ["cargo", "test", "-p", "rustic-fs", "--test", "fs6_image", "--locked"]
    result = subprocess.run(command, cwd=environment.ROOT, capture_output=True, text=True,
                            env={**os.environ, "RUSTIC_FS6_EXPORT": str(images)})
    if result.returncode != 0:
        raise SystemExit("image test failed:\n" + result.stdout + result.stderr)
    missing = [name for name in ("v5-source.img", "v6-provisioned.img", "v6-migrated.img")
               if not (images / name).is_file()]
    if missing:
        raise SystemExit(f"the image test did not export {missing}")
    return images, {"command": command, "stdout_tail": result.stdout.strip().splitlines()[-3:]}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def refusal(name, data, mutate, evidence):
    """Record that the independent reader refuses a damaged image."""
    damaged = bytearray(data)
    mutate(damaged)
    try:
        oracle6.snapshot(bytes(damaged))
    except AssertionError as error:
        evidence.append({"case": name, "refused": str(error)})
        return
    raise SystemExit(f"the v6 reader accepted a damaged image: {name}")


def verify(images, output):
    provisioned = oracle6.snapshot((images / "v6-provisioned.img").read_bytes())
    migrated = oracle6.snapshot((images / "v6-migrated.img").read_bytes())
    _, source = oracle.snapshot(images / "v5-source.img")

    # Provision: one file that no v5 record could describe, stored in runs.
    node = provisioned["nodes"].get(5)
    if node is None or node["length"] != BIG or node["version"] != 2:
        raise SystemExit("provisioned file record does not describe the written bytes")
    if len(node["extents"]) < 2:
        raise SystemExit("the 200 kB fixture must span more than one run")
    if provisioned["files"].get((4, "artifact")) != PATTERN:
        raise SystemExit("independent read of the provisioned payload differs")
    if provisioned["used_sectors"] != (BIG + 511) // 512 or provisioned["receipts"]:
        raise SystemExit("provisioned volume accounting or receipts are wrong")
    roots = {node["name"] for node in provisioned["nodes"].values() if node["kind"] == "directory"}
    if roots != {"system", "data", "config", "workspaces"}:
        raise SystemExit("the provisioned volume does not carry its four roots")

    # Migration: the v5 reader describes the source, the v6 reader the result.
    if migrated["files"] != source["files"]:
        raise SystemExit("the migration changed the files the v5 reader sees")
    for identity, record in source["nodes"].items():
        if migrated["nodes"][identity]["version"] != record["version"]:
            raise SystemExit("the migration changed a committed version")
    if migrated["nodes"][1]["name"] != "system" or migrated["receipt_epoch"] == 0:
        raise SystemExit("the migrated volume lost a root or its receipt identity")

    evidence = []
    raw = (images / "v6-provisioned.img").read_bytes()
    high = oracle6.PAYLOAD_SECTOR + max(
        run[0] + run[1] for record in provisioned["nodes"].values() for run in record["extents"])
    # The header names the active generation, so damage is applied where the
    # reader must look rather than at a fixed guess.
    active = provisioned["active"]
    at = oracle6.HEADER_SECTOR * 512
    refusal("header magic", raw, lambda data: data.__setitem__(at + 3, 0x39), evidence)
    refusal("header checksum", raw, lambda data: data.__setitem__(at + 12, data[at + 12] ^ 1), evidence)
    nodes_at = oracle6.nodes_sector(active) * 512
    refusal("node table", raw, lambda data: data.__setitem__(nodes_at + 5, data[nodes_at + 5] ^ 1), evidence)
    map_at = oracle6.map_sector(active) * 512
    refusal("free-space map", raw, lambda data: data.__setitem__(map_at, data[map_at] ^ 1), evidence)
    receipts_at = oracle6.receipts_sector(active) * 512
    refusal("receipt block", raw, lambda data: data.__setitem__(receipts_at + 4, data[receipts_at + 4] ^ 1), evidence)
    refusal("truncated payload", raw[: (high - 1) * 512], lambda data: None, evidence)

    # Payload bytes carry no checksum: the reader returns them and the consumer
    # comparison is what rejects them, which is recorded rather than implied.
    start = (oracle6.PAYLOAD_SECTOR + provisioned["nodes"][5]["extents"][0][0]) * 512
    flipped = bytearray(raw)
    flipped[start] ^= 1
    if oracle6.snapshot(bytes(flipped))["files"][(4, "artifact")] == PATTERN:
        raise SystemExit("a flipped payload byte was not visible to the reader")

    report = {
        "provisioned": {"bytes": BIG, "extents": len(node["extents"]),
                        "used_sectors": provisioned["used_sectors"],
                        "free_sectors": provisioned["free_sectors"],
                        "sequence": provisioned["sequence"]},
        "migrated": {"files": len(migrated["files"]), "used_sectors": migrated["used_sectors"],
                     "sequence": migrated["sequence"],
                     "receipt_epoch": migrated["receipt_epoch"]},
        "source_files": sorted(name for _, name in source["files"]),
        "refusals": evidence,
        "images": {path.name: {"bytes": path.stat().st_size, "sha256": digest(path)}
                   for path in sorted(images.iterdir())},
    }
    (output / "evidence.json").write_text(json.dumps({
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=environment.ROOT).decode().strip(),
        "worktree_status": subprocess.check_output(["git", "status", "--porcelain"], cwd=environment.ROOT).decode(),
        **report,
    }, indent=2) + "\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    images, rust = export(arguments.output)
    report = verify(images, arguments.output)
    print(f"v6 images verified independently: provisioned {report['provisioned']['bytes']} bytes "
          f"in {report['provisioned']['extents']} runs, migration kept "
          f"{report['migrated']['files']} file(s), {len(report['refusals'])} damaged images refused")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
