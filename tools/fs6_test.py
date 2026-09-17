# SPDX-License-Identifier: Apache-2.0
"""Verify v6 volume images with an independent reader (#51).

`tools/volume` (package `rustic-volume`) provisions, seeds and migrates real
images, and `cargo test -p rustic-fs --test fs6_image` exports a provisioned image
whose payload spans runs. This suite reads those images with
`terminal_support.oracle6`, a reader written from the format description rather
than from the Rust code, compares the migration with what `terminal_support.oracle`
(the existing v5 reader) says about the source image, checks the tool's own JSON
report against both readers, and records what a reader refuses.
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
LINEAGE = "03030303030303030303030303030303"
SEEDED_NAME = (5, "notes.txt")
SEEDED_ID = 6
SEEDED_BYTES = b"a v5 record"
BIG = 200_000
PATTERN = bytes(index % 251 for index in range(BIG))


def cargo(arguments):
    return subprocess.run(["cargo", *arguments, "--locked"], cwd=environment.ROOT,
                          capture_output=True, text=True)


def tool(output, *arguments):
    """Run the host volume tool and keep its JSON line."""
    binary = environment.ROOT / "target/debug/rustic-volume"
    result = subprocess.run([binary, *arguments], cwd=environment.ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(f"rustic-volume {arguments[0]} failed: {result.stdout}{result.stderr}")
    (output / f"{arguments[0]}.json").write_text(result.stdout)
    return json.loads(result.stdout)


def tool_refusal(output, name, *arguments):
    """The tool must refuse an image it cannot act on, with a reason."""
    binary = environment.ROOT / "target/debug/rustic-volume"
    result = subprocess.run([binary, *arguments], cwd=environment.ROOT, capture_output=True, text=True)
    if result.returncode == 0:
        raise SystemExit(f"rustic-volume {arguments[0]} accepted {name}")
    return {"case": name, "refused": result.stderr.strip()}


def export(output):
    """Build the host tool and run the Rust image test, which exports images."""
    for command in (["build", "-p", "rustic-volume"],):
        result = cargo(command)
        if result.returncode != 0:
            raise SystemExit(f"cargo {command} failed:\n{result.stdout}{result.stderr}")
    images = output / "images"
    shutil.rmtree(images, ignore_errors=True)
    images.mkdir(parents=True)
    result = subprocess.run(["cargo", "test", "-p", "rustic-fs", "--test", "fs6_image", "--locked"],
                            cwd=environment.ROOT, capture_output=True, text=True,
                            env={**os.environ, "RUSTIC_FS6_EXPORT": str(images)})
    if result.returncode != 0:
        raise SystemExit("image export failed:\n" + result.stdout + result.stderr)
    for name in ("v5-source.img", "v6-provisioned.img", "v6-tracked.img"):
        if not (images / name).is_file():
            raise SystemExit(f"the image test did not export {name}")
    return images


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


def verify(output, images):
    work = output / "volumes"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)

    # The tool provisions a volume the independent reader must recognise.
    fresh = work / "fresh.img"
    provisioned = tool(output, "provision", str(fresh), LINEAGE)
    state = oracle6.snapshot(fresh.read_bytes())
    if provisioned["free_sectors"] != oracle6.DATA_SECTORS or state["used_sectors"]:
        raise SystemExit("a fresh v6 volume must hold no payload")
    if state["receipt_epoch"] != 1 or state["receipt_lineage"].hex() != LINEAGE:
        raise SystemExit("a fresh v6 volume did not carry its receipt identity")
    roots = sorted(node["name"] for node in state["nodes"].values() if node["kind"] == "directory")
    if roots != ["config", "data", "system", "workspaces"]:
        raise SystemExit("the provisioned volume does not carry its four roots")

    # The tool seeds a v5 volume, the v5 reader confirms it, the tool migrates it,
    # and the v6 reader must see exactly what the v5 reader saw.
    source = work / "source.img"
    seeded = tool(output, "seed", str(source))
    _, v5 = oracle.snapshot(source)
    if v5["files"] != {SEEDED_NAME: SEEDED_BYTES, (4, "empty.bin"): b""}:
        raise SystemExit("the v5 reader disagrees with the seeded volume")
    migrated = tool(output, "migrate", str(source), LINEAGE)
    state = oracle6.snapshot(source.read_bytes())
    if state["files"] != v5["files"]:
        raise SystemExit("the migration changed the files the v5 reader sees")
    for identity, record in v5["nodes"].items():
        if state["nodes"][identity]["version"] != record["version"]:
            raise SystemExit("the migration changed a committed version")
    if (migrated["files"], migrated["bytes"]) != (2, len(SEEDED_BYTES)):
        raise SystemExit("the migration report disagrees with the volume")
    if migrated["next"] != seeded["empty"]["id"] + 1:
        raise SystemExit("the reported identity watermark is not the seeded one")

    # The tool's own report and the independent reader must describe one volume.
    reported = tool(output, "report", str(source))
    expect = {"sequence": state["sequence"], "active": state["active"],
              "used_sectors": state["used_sectors"], "free_sectors": state["free_sectors"],
              "lineage": LINEAGE, "epoch": state["receipt_epoch"], "receipts": len(state["receipts"])}
    for key, value in expect.items():
        if reported[key] != value:
            raise SystemExit(f"the tool report and the reader disagree on {key}")
    seen = {node["id"]: node for node in reported["nodes"]}
    if seen[SEEDED_ID]["name"] != SEEDED_NAME[1] or seen[SEEDED_ID]["length"] != len(SEEDED_BYTES):
        raise SystemExit("the tool report lost the migrated file")
    if [tuple(run) for run in seen[SEEDED_ID]["extents"]] != state["nodes"][SEEDED_ID]["extents"]:
        raise SystemExit("the tool report and the reader disagree on the runs")

    # A payload larger than any v5 file, from the Rust image test, read by the
    # independent reader: runs, exact accounting and the bytes themselves.
    big = oracle6.snapshot((images / "v6-provisioned.img").read_bytes())
    node = big["nodes"].get(5)
    if node is None or node["length"] != BIG or node["version"] != 2 or len(node["extents"]) < 2:
        raise SystemExit("the exported 200 kB fixture is not the expected record")
    if big["files"].get((4, "artifact")) != PATTERN or big["receipts"]:
        raise SystemExit("independent read of the exported payload differs")
    if big["used_sectors"] != (BIG + 511) // 512:
        raise SystemExit("the exported volume accounting is wrong")

    # A tracked write publishes the bytes, the version and the receipt that names
    # them together, and the independent reader checks the binding.
    tracked = oracle6.snapshot((images / "v6-tracked.img").read_bytes())
    receipt, = tracked["receipts"]
    if (receipt["id"], receipt["previous"], receipt["committed"], receipt["length"]) != (5, 1, 2, 4096):
        raise SystemExit("the retained receipt does not describe the tracked write")
    if tracked["nodes"][5]["version"] != receipt["committed"]:
        raise SystemExit("the receipt and the committed version disagree")
    if tracked["files"].get((4, "artifact")) != bytes(index % 251 for index in range(4096)):
        raise SystemExit("independent read of the tracked payload differs")

    evidence = [tool_refusal(output, "an existing v6 volume", "migrate", str(fresh), LINEAGE),
                tool_refusal(output, "an image that is not a volume", "report", str(images / "v5-source.img"))]
    raw = (images / "v6-provisioned.img").read_bytes()
    high = oracle6.PAYLOAD_SECTOR + max(
        run[0] + run[1] for record in big["nodes"].values() for run in record["extents"])
    # The header names the active generation, so damage goes where the reader looks.
    active = big["active"]
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
    # comparison is what rejects them, recorded rather than implied.
    start = (oracle6.PAYLOAD_SECTOR + node["extents"][0][0]) * 512
    flipped = bytearray(raw)
    flipped[start] ^= 1
    if oracle6.snapshot(bytes(flipped))["files"][(4, "artifact")] == PATTERN:
        raise SystemExit("a flipped payload byte was not visible to the reader")

    report = {
        "provisioned": {"free_sectors": provisioned["free_sectors"], "sequence": provisioned["sequence"]},
        "seeded": seeded,
        "migrated": migrated,
        "exported": {"bytes": BIG, "extents": len(node["extents"]), "used_sectors": big["used_sectors"]},
        "tracked": {"receipts": len(tracked["receipts"]), "committed": receipt["committed"],
                    "length": receipt["length"]},
        "refusals": evidence,
        "images": {path.name: {"bytes": path.stat().st_size, "sha256": digest(path)}
                   for path in sorted(images.iterdir())},
        "volumes": {path.name: digest(path) for path in sorted(work.iterdir())},
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
    images = export(arguments.output)
    report = verify(arguments.output, images)
    print(f"v6 images verified independently: {report['exported']['bytes']} bytes in "
          f"{report['exported']['extents']} runs, migration kept {report['migrated']['files']} file(s), "
          f"{len(report['refusals'])} damaged or inapplicable images refused")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
