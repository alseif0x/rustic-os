# SPDX-License-Identifier: Apache-2.0
"""Verify V7 workspace images with an independent reader (#51).

`tools/volume` (package `rustic-volume`) creates the application fixture with
`seed7`, and `cargo test -p rustic-fs --test fs7_image` exports full-size images
that hold every retained record state, a removal with a retained snapshot, an
executed admission, a noncontiguous file and an advanced retry epoch. The same
tool also builds three disposable v5 sources with `seed5-history` and converts
each with `migrate7`. This suite reads each v7 image with
`terminal_support.oracle7`, written from `docs/WORKSPACE-FORMAT7.md` rather than
from the Rust code, checks the bytes against payload patterns it regenerates
itself, compares the reader with the tool's own verified `report7`, compares
every migrated image with the independent v5 reader's view of its source (whose
digest must not change), and then damages copies of an image to record, case by
case, what the reader and the Rust mount each accept or refuse. Any
disagreement between the two is a failure of this suite.
"""
import argparse
import hashlib
import json
import os
import random
import shutil
import struct
import subprocess
import zlib
from pathlib import Path

import application
import environment
from terminal_support import oracle, oracle7

DEFAULT_OUTPUT = environment.ROOT / "artifacts/fs7"
EXPORTS = ("v7-history.img", "v7-fragmented.img", "v7-maintained.img")
SEED_LINEAGE = "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f"
MIGRATION_LINEAGE = "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e"
MIGRATION_SETS = ("receipts", "admissions", "completed")
# v5 record states as `terminal_support.oracle` names them, in v7 terms.
V5_STATES = {None: "direct_committed", "admitted": "admitted", "cancelled": "cancelled",
             "committed": "admitted_committed"}
FIXTURE_ELF_BYTES = 300_000
SECTOR = oracle7.SECTOR


def pattern(seed, length):
    """The payload pattern `crates/fs/tests/fs7_image.rs` writes, regenerated here."""
    return bytes((index * (2 * seed + 1) + seed) % 251 for index in range(length))


def sha(data):
    return hashlib.sha256(data).hexdigest()


def shell_pattern(seed, size):
    """The V7 shell's `replace-pattern-v7` bytes: s*31 + 7*i + i//509 mod 256."""
    return bytes((seed * 31 + 7 * index + index // 509) & 0xFF for index in range(size))


def file_sha(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def run(command, **options):
    return subprocess.run(command, cwd=environment.ROOT, capture_output=True, text=True, **options)


def volume_tool(*arguments):
    """Run `rustic-volume`; return (parsed JSON or None, refusal text or None)."""
    result = run([str(environment.ROOT / "target/debug/rustic-volume"), *map(str, arguments)])
    if result.returncode == 0:
        return json.loads(result.stdout), None
    return None, result.stderr.strip() or f"exit {result.returncode}"


def export(output):
    """Build the host tool and let the Rust image test write its images."""
    result = run(["cargo", "build", "-p", "rustic-volume", "--locked"])
    if result.returncode != 0:
        raise SystemExit("cargo build -p rustic-volume failed:\n" + result.stdout + result.stderr)
    images = output / "images"
    shutil.rmtree(images, ignore_errors=True)
    images.mkdir(parents=True)
    result = run(["cargo", "test", "-p", "rustic-fs", "--test", "fs7_image", "--locked"],
                 env={**os.environ, "RUSTIC_FS7_EXPORT": str(images)})
    if result.returncode != 0:
        raise SystemExit("V7 image export failed:\n" + result.stdout + result.stderr)
    for name in EXPORTS:
        if not (images / name).is_file():
            raise SystemExit(f"the image test did not export {name}")
    return images


def fixture_inputs(directory):
    """A minimal loadable x86-64 ELF over several payload sectors and its manifest."""
    elf = bytearray(pattern(21, FIXTURE_ELF_BYTES))
    elf[:64] = bytes(64)
    elf[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<HHIQQ", elf, 16, 2, 62, 1, 0x400000, 64)
    struct.pack_into("<HHH", elf, 52, 64, 56, 1)
    struct.pack_into("<IIQQQQQQ", elf, 64, 1, 5, 0, 0x400000, 0x400000,
                     FIXTURE_ELF_BYTES, FIXTURE_ELF_BYTES, 0x1000)
    elf = bytes(elf)
    manifest = application.encode({"schema": 2, "identity": "org.rusticos.fs7-fixture",
                                   "executable": "fixture.elf", "version": [0, 1, 0],
                                   "process_abi": 65536, "ipc_version": 1, "requests": []},
                                  hashlib.sha256(elf).digest())
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "fixture.elf").write_bytes(elf)
    (directory / "fixture.manifest").write_bytes(manifest)
    return directory / "fixture.elf", directory / "fixture.manifest"


def write_sparse(path, data):
    """Write a full-size image without materialising its all-zero payload."""
    step = 1 << 16
    zero = bytes(step)
    view = memoryview(data)
    with open(path, "wb") as handle:
        handle.truncate(len(data))
        for at in range(0, len(data), step):
            chunk = view[at:at + step]
            if chunk != zero[:len(chunk)]:
                handle.seek(at)
                handle.write(chunk)


def compare_report(state, report, label):
    """The tool's verified mount and the independent reader must describe one volume."""
    expect = {"lineage": state["lineage"], "sequence": state["sequence"], "epoch": state["epoch"],
              "generation": state["generation"], "next": state["next"], "recovered": state["recovered"],
              "free_sectors": state["free_sectors"], "objects": len(state["nodes"])}
    for key, value in expect.items():
        if report.get(key) != value:
            raise SystemExit(f"{label}: report7 and oracle7 disagree on {key}: {report.get(key)!r} != {value!r}")
    ours = sorted((node["id"], node["parent"], node["version"], node["length"], node["kind"], node["name"])
                  for node in state["nodes"].values())
    theirs = sorted((node["id"], node["parent"], node["version"], node["size"], node["kind"], node["name"])
                    for node in report["nodes"])
    if ours != theirs:
        raise SystemExit(f"{label}: report7 and oracle7 disagree on the live nodes")
    fields = ("slot", "state", "cause", "subject", "workspace", "object", "instance", "epoch", "key",
              "previous", "committed", "admission", "terminal")
    ours = [tuple(record[field] for field in fields) + (record["length"],) for record in state["records"]]
    theirs = [tuple(record[field] for field in fields) + (record["size"],) for record in report["records"]]
    if ours != theirs:
        raise SystemExit(f"{label}: report7 and oracle7 disagree on the retained records")


def read_image(path):
    data = path.read_bytes()
    state = oracle7.snapshot(data)
    report, refused = volume_tool("report7", path)
    if refused:
        raise SystemExit(f"report7 refused {path.name}: {refused}")
    compare_report(state, report, path.name)
    return data, state


def expect_files(state, expected, label):
    """`expected` maps path -> (version, bytes)."""
    if set(state["contents"]) != set(expected):
        raise SystemExit(f"{label}: files differ: {sorted(state['contents'])}")
    listed = {entry["path"]: entry for entry in state["files"]}
    for path, (version, content) in expected.items():
        if state["contents"][path] != content or listed[path]["sha256"] != sha(content):
            raise SystemExit(f"{label}: {path} bytes differ from the regenerated pattern")
        if listed[path]["version"] != version or listed[path]["size"] != len(content):
            raise SystemExit(f"{label}: {path} is not at version {version} with {len(content)} bytes")


def expect_records(state, expected, label):
    """`expected` is a list of (state, cause, target path or None, aliases_live, bytes)."""
    seen = [(record["state"], record["cause"], record["target"] and record["target"]["path"],
             record["aliases_live"], record["sha256"]) for record in state["records"]]
    want = [(state_, cause, target, alias, sha(content)) for state_, cause, target, alias, content in expected]
    if seen != want:
        raise SystemExit(f"{label}: retained records differ:\n{seen}\n!=\n{want}")


def verify_images(output, images):
    summary = {}
    # Application fixture written by the host tool.
    inputs = fixture_inputs(output / "inputs")
    seeded_path = output / "volumes/seed7.img"
    seeded_path.parent.mkdir(parents=True, exist_ok=True)
    seeded_path.unlink(missing_ok=True)
    seeded, refused = volume_tool("seed7", seeded_path, SEED_LINEAGE, *inputs)
    if refused:
        raise SystemExit(f"seed7 refused the fixture: {refused}")
    _, state = read_image(seeded_path)
    expect_files(state, {
        "/workspaces/application/fixture.elf": (seeded["elf"]["version"], inputs[0].read_bytes()),
        "/workspaces/application/fixture.manifest": (seeded["manifest"]["version"], inputs[1].read_bytes()),
    }, "seed7")
    expect_records(state, [
        ("direct_committed", None, "/workspaces/application/fixture.elf", True, inputs[0].read_bytes()),
        ("direct_committed", None, "/workspaces/application/fixture.manifest", True, inputs[1].read_bytes()),
    ], "seed7")
    summary["seed7"] = {"files": state["files"], "used_sectors": state["used_sectors"],
                        "sequence": state["sequence"]}

    app = "/workspaces/app/"
    _, history = read_image(images / "v7-history.img")
    expect_files(history, {app + "alpha": (15, pattern(6, 10_000)), app + "gamma": (5, b""),
                           app + "delta": (16, pattern(7, 5000))}, "history")
    expect_records(history, [
        ("direct_committed", None, app + "alpha", False, pattern(1, 200_000)),
        ("direct_committed", None, app + "alpha", False, pattern(2, 4096)),
        ("direct_committed", None, None, False, pattern(3, 1000)),
        ("admitted", None, app + "gamma", False, pattern(4, 3000)),
        ("cancelled", "requested", app + "delta", False, pattern(5, 700)),
        ("admitted_committed", None, app + "alpha", True, pattern(6, 10_000)),
        ("direct_committed", None, app + "delta", True, pattern(7, 5000)),
    ], "history")

    _, fragmented = read_image(images / "v7-fragmented.img")
    expect_files(fragmented, {"/data/frag": (3, pattern(8, 3000)), "/data/other": (4, pattern(10, 2000))},
                 "fragmented")
    expect_records(fragmented, [("direct_committed", None, "/data/frag", True, pattern(8, 3000)),
                                ("direct_committed", None, "/data/other", True, pattern(10, 2000))],
                   "fragmented")
    if fragmented["nodes"][5]["runs"] != [(10, 2), (0, 3), (20, 1)]:
        raise SystemExit("fragmented: the three noncontiguous runs were not read in record order")

    _, maintained = read_image(images / "v7-maintained.img")
    if maintained["epoch"] != 2:
        raise SystemExit("maintained: retention did not advance the epoch")
    expect_files(maintained, {"/config/settings/value": (7, pattern(13, 800))}, "maintained")
    expect_records(maintained, [("direct_committed", None, "/config/settings/value", True, pattern(13, 800))],
                   "maintained")
    for name, state in (("history", history), ("fragmented", fragmented), ("maintained", maintained)):
        summary[name] = {"sequence": state["sequence"], "generation": state["generation"],
                         "epoch": state["epoch"], "files": state["files"],
                         "records": [{key: record[key] for key in ("slot", "state", "cause", "object", "previous",
                                                                   "committed", "admission", "terminal", "length",
                                                                   "runs", "aliases_live", "sha256")}
                                     for record in state["records"]],
                         "used_sectors": state["used_sectors"]}
    return summary, history


def compare_migration(source_view, state, seeded, label):
    """A migrated image must hold exactly the v5 source's identity, files and history."""
    if (state["lineage"], state["sequence"], state["epoch"]) != \
            (source_view["lineage"], source_view["sequence"], source_view["epoch"]):
        raise SystemExit(f"{label}: lineage, sequence or epoch differs from the v5 source")
    # The v5 reader lists files only; directories are covered by report7's comparison.
    if {item["id"]: item["version"] for item in state["files"]} != \
            {identity: node["version"] for identity, node in source_view["nodes"].items()}:
        raise SystemExit(f"{label}: live file identities or versions differ from the v5 source")
    for item in state["files"]:
        if item["sha256"] != sha(source_view["nodes"][item["id"]]["content"]):
            raise SystemExit(f"{label}: {item['path']} bytes differ from the v5 source")
    fields = ("subject", "workspace", "object", "instance", "epoch", "key", "previous", "committed",
              "admission", "terminal", "state", "cause", "sha256")
    ours = [tuple(record[field] for field in fields) for record in state["records"]]
    theirs = [(record["subject"], record["workspace"], record["id"], record["instance"], record["epoch"],
               record["key"], record["previous"], record["committed"], record.get("admission", 0),
               record.get("terminal", record["committed"]), V5_STATES[record.get("state")],
               record.get("prevention"), record["sha256"]) for record in source_view["records"]]
    if ours != theirs:
        raise SystemExit(f"{label}: retained records differ from the v5 source:\n{ours}\n!=\n{theirs}")
    wanted = [(record["state"], record["cause"], record["subject"], record["key"],
               sha(shell_pattern(record["seed"], record["size"]))) for record in seeded["records"]]
    if [(record["state"], record["cause"], record["subject"], record["key"], record["sha256"])
            for record in state["records"]] != wanted:
        raise SystemExit(f"{label}: retained records are not the seeded shell patterns")


def verify_migrations(output):
    """Seed each v5 history set, migrate it and compare both readers' views."""
    summary = {}
    volumes = output / "volumes"
    volumes.mkdir(parents=True, exist_ok=True)
    for name in MIGRATION_SETS:
        source, target = volumes / f"migrate-{name}.v5", volumes / f"migrate-{name}.v7"
        source.unlink(missing_ok=True)
        target.unlink(missing_ok=True)
        seeded, refused = volume_tool("seed5-history", source, MIGRATION_LINEAGE, name)
        if refused:
            raise SystemExit(f"seed5-history {name} refused: {refused}")
        before = file_sha(source)
        _, source_view = oracle.snapshot(source)
        migrated, refused = volume_tool("migrate7", source, target, MIGRATION_LINEAGE)
        if refused:
            raise SystemExit(f"migrate7 {name} refused: {refused}")
        if (migrated["source_sha256_before"], migrated["source_sha256_after"], file_sha(source)) != (before,) * 3:
            raise SystemExit(f"migrate7 {name}: the source digest changed")
        _, state = read_image(target)
        compare_migration(source_view, state, seeded, f"migrate7 {name}")
        target_digest = file_sha(target)
        again, refused = volume_tool("migrate7", source, target, MIGRATION_LINEAGE)
        if again is not None or file_sha(target) != target_digest or file_sha(source) != before:
            raise SystemExit(f"migrate7 {name}: an existing target was not refused untouched")
        summary[name] = {"source_sha256": before, "sequence": state["sequence"], "epoch": state["epoch"],
                         "recovered": state["recovered"], "files": state["files"],
                         "records": [{key: record[key] for key in ("slot", "state", "cause", "subject", "object",
                                                                   "key", "previous", "committed", "admission",
                                                                   "terminal", "length", "aliases_live", "sha256")}
                                     for record in state["records"]],
                         "existing_target": refused}
        source.unlink()
        target.unlink()
    return summary


# Damage writers: these produce checksum-valid structural faults, so a refusal
# shows the structural rule, not only a checksum, was enforced.

def node_offset(data, generation, identity):
    base = oracle7.nodes_sector(generation) * SECTOR
    for index in range(oracle7.NODES):
        at = base + index * oracle7.NODE_BYTES
        if struct.unpack_from("<I", data, at)[0] == identity and data[at + 20]:
            return at
    raise SystemExit(f"node {identity} not found")


def record_offset(generation, slot):
    return oracle7.receipts_sector(generation) * SECTOR + slot * oracle7.RECORD_BYTES


def reseal_node(data, at):
    struct.pack_into("<I", data, at + 124, zlib.crc32(data[at:at + 124]))


def reseal_record(data, at):
    struct.pack_into("<I", data, at + 188, zlib.crc32(data[at:at + 188]))


def reseal_header(data, generation):
    header = oracle7.header_sector(generation) * SECTOR
    for field, first, count in ((48, oracle7.nodes_sector(generation), oracle7.NODES_SECTORS),
                                (52, oracle7.map_sector(generation), oracle7.MAP_SECTORS),
                                (56, oracle7.receipts_sector(generation), oracle7.RECEIPTS_SECTORS)):
        struct.pack_into("<I", data, header + field, zlib.crc32(data[first * SECTOR:(first + count) * SECTOR]))
    data[header + 508:header + 512] = bytes(4)
    struct.pack_into("<I", data, header + 508, zlib.crc32(data[header:header + SECTOR]))


def set_map_bit(data, generation, sector, value):
    at = oracle7.map_sector(generation) * SECTOR + sector // 8
    if value:
        data[at] |= 1 << (sector % 8)
    else:
        data[at] &= ~(1 << (sector % 8)) & 0xFF


def payload_byte(sector, offset=0):
    return (oracle7.PAYLOAD_SECTOR + sector) * SECTOR + offset


def damage_cases(history):
    """(name, mutate(bytearray) -> bytearray or None, expected outcome, accepted check)."""
    newest = history["generation"]
    older = 1 - newest
    alpha, delta, gamma = (history["nodes"][identity] for identity in (6, 9, 8))
    retained = history["records"][0]["runs"][0][0]
    removed = history["records"][2]  # beta's snapshot, whose object was removed
    admitted = history["records"][3]  # gamma's unresolved admission
    if (removed["target"], admitted["state"]) != (None, "admitted"):
        raise SystemExit("history image records are not in the order the damage cases expect")
    removed_run = removed["runs"][0]
    app = "/workspaces/app/"

    def flip(at):
        def mutate(data):
            data[at] ^= 0x01
        return mutate

    def older_generation(state):
        expect_files(state, {app + "alpha": (15, pattern(6, 10_000)), app + "gamma": (5, b""),
                             app + "delta": (6, b"")}, "fallback")
        if (state["generation"], state["sequence"], state["recovered"], len(state["records"])) != (older, 15, True, 6):
            raise SystemExit("fallback did not select the complete older generation")

    def unchanged(state):
        expect_files(state, {app + "alpha": (15, pattern(6, 10_000)), app + "gamma": (5, b""),
                             app + "delta": (16, pattern(7, 5000))}, "padding")

    def both_headers(data):
        for slot in (0, 1):
            data[oracle7.header_sector(slot) * SECTOR + 20] ^= 0x01

    def older_sequence(data):
        at = oracle7.header_sector(older) * SECTOR
        struct.pack_into("<Q", data, at + 36, struct.unpack_from("<Q", data, at + 36)[0] - 1)
        reseal_header(data, older)

    def older_watermark(data):
        at = oracle7.header_sector(older) * SECTOR
        struct.pack_into("<I", data, at + 44, history["next"] + 1)
        reseal_header(data, older)

    def map_leak(data):
        set_map_bit(data, newest, 100_000, True)
        reseal_header(data, newest)

    def double_owner(data):
        at = record_offset(newest, removed["slot"])
        struct.pack_into("<I", data, at + 88, alpha["runs"][0][0])
        for sector in range(removed_run[0], removed_run[0] + removed_run[1]):
            set_map_bit(data, newest, sector, False)
        reseal_record(data, at)
        reseal_header(data, newest)

    def record_crc(data):
        at = record_offset(newest, admitted["slot"])
        data[at + 40] ^= 0x01
        reseal_header(data, newest)

    def orphan(data):
        at = node_offset(data, newest, gamma["id"])
        struct.pack_into("<I", data, at + 4, 7)  # beta's identity, removed in this generation
        reseal_node(data, at)
        reseal_header(data, newest)

    def other_epoch(data):
        at = record_offset(newest, admitted["slot"])
        struct.pack_into("<Q", data, at + 24, 2)
        reseal_record(data, at)
        reseal_header(data, newest)

    def truncate_payload(data):
        return data[:payload_byte(alpha["runs"][0][0] + 1)]

    def truncate_tail(data):
        return data[:-SECTOR]

    def extend(data):
        return data + bytes(SECTOR)

    tail = delta["length"] % SECTOR
    padding_sector = delta["runs"][-1][0] + delta["runs"][-1][1] - 1
    return [
        ("live payload byte flipped", flip(payload_byte(alpha["runs"][0][0], 7)), "refused", None),
        ("retained-only snapshot byte flipped", flip(payload_byte(retained, 100)), "refused", None),
        ("padding after a file's last byte flipped", flip(payload_byte(padding_sector, tail + 8)),
         "accepted", unchanged),
        ("newest header checksum broken, older copy valid",
         flip(oracle7.header_sector(newest) * SECTOR + 20), "accepted", older_generation),
        ("both header checksums broken", both_headers, "refused", None),
        ("node table named by the valid newest header damaged",
         flip(oracle7.nodes_sector(newest) * SECTOR + 5), "refused", None),
        ("older header resealed with a non-adjacent sequence", older_sequence, "refused", None),
        ("older header resealed with a higher identity watermark", older_watermark, "refused", None),
        ("allocation map marks an unowned sector (resealed)", map_leak, "refused", None),
        ("retained snapshot overlaps a live file (resealed)", double_owner, "refused", None),
        ("retained record checksum broken (aggregate resealed)", record_crc, "refused", None),
        ("retained record region aggregate broken", flip(record_offset(newest, admitted["slot"]) + 40),
         "refused", None),
        ("node parent names a removed identity (resealed)", orphan, "refused", None),
        ("retained record from another retry epoch (resealed)", other_epoch, "refused", None),
        # Rust side for these three is report7's exact-size precheck, not the mount;
        # the mount's own Io refusal of a missing referenced sector is covered by
        # crates/fs/tests/fs7_image.rs.
        ("image truncated inside a live payload (size precheck)", truncate_payload, "refused", None),
        ("image truncated by one unused tail sector (size precheck)", truncate_tail, "refused", None),
        ("image one sector longer than the volume (size precheck)", extend, "refused", None),
    ]


def verify_damage(output, source, history):
    base = source.read_bytes()
    work = output / "damaged"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    evidence = []
    for index, (name, mutate, expected, check) in enumerate(damage_cases(history)):
        data = bytearray(base)
        data = mutate(data) or data
        path = work / f"case-{index:02d}.img"
        write_sparse(path, data)
        try:
            state, oracle_refusal = oracle7.snapshot(data), None
        except oracle7.Corrupt as error:
            state, oracle_refusal = None, str(error)
        report, rust_refusal = volume_tool("report7", path)
        case = {"case": name, "expected": expected,
                "oracle7": {"refused": oracle_refusal} if oracle_refusal else {"accepted": True},
                "rust": {"refused": rust_refusal} if rust_refusal else {"accepted": True}}
        if (oracle_refusal is None) != (rust_refusal is None):
            raise SystemExit(f"oracle7 and the Rust mount disagree on '{name}': {case}")
        outcome = "refused" if oracle_refusal else "accepted"
        if outcome != expected:
            raise SystemExit(f"'{name}' was {outcome} by both readers; expected {expected}: {case}")
        if state is not None:
            compare_report(state, report, name)
            check(state)
            case["selected"] = {"generation": state["generation"], "sequence": state["sequence"],
                                "recovered": state["recovered"], "rejected_headers": state["rejected_headers"]}
        case["sha256"] = sha(bytes(data))
        evidence.append(case)
        path.unlink()
    work.rmdir()
    return evidence


# Field offsets a deterministic differential sweep perturbs, from the format tables.
SWEEP_FIELDS = {
    "record": [(0, "Q"), (8, "I"), (12, "I"), (16, "Q"), (24, "Q"), (32, "Q"), (40, "Q"), (48, "Q"), (56, "Q"),
               (64, "Q"), (72, "I"), (80, "B"), (81, "B"), (82, "B"), (88, "I"), (92, "I"), (96, "I"), (100, "I")],
    "node": [(0, "I"), (4, "I"), (8, "Q"), (16, "I"), (20, "B"), (21, "B"), (22, "B"), (23, "B"), (24, "I"),
             (28, "I"), (32, "I"), (36, "I")],
    "header": [(28, "Q"), (36, "Q"), (44, "I")],
}
LIMITS = {"B": 0xFF, "I": 0xFFFF_FFFF, "Q": 0xFFFF_FFFF_FFFF_FFFF}


def differential_sweep(output, images, count, seed):
    """Resealed small field perturbations; oracle7 and the Rust mount must agree on each.

    Every mutated structure gets valid record and aggregate CRCs, so the verdict
    comes from the structural and temporal rules rather than from a checksum.
    """
    rng = random.Random(seed)
    work = output / "sweep"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    totals = {"accepted": 0, "refused": 0}
    # Only headers and generation metadata change, so one image copy is reused and
    # just that prefix is restored and rewritten for each case.
    prefix = oracle7.PAYLOAD_SECTOR * SECTOR
    path = work / "case.img"
    for name in EXPORTS:
        base = (images / name).read_bytes()
        generation = oracle7.snapshot(base)["generation"]
        data = bytearray(base)
        write_sparse(path, data)
        for _ in range(count):
            data[:prefix] = base[:prefix]
            for _ in range(rng.choice((1, 1, 2))):
                kind = rng.choice(("record", "record", "node", "header", "map"))
                if kind == "map":
                    set_map_bit(data, generation, rng.randrange(600), rng.random() < 0.5)
                    continue
                if kind == "record":
                    at = record_offset(generation, rng.randrange(oracle7.RETAINED))
                    size = oracle7.RECORD_BYTES
                elif kind == "node":
                    at = oracle7.nodes_sector(generation) * SECTOR + rng.randrange(10) * oracle7.NODE_BYTES
                    size = oracle7.NODE_BYTES
                else:
                    at = oracle7.header_sector(rng.randrange(2)) * SECTOR
                    size = SECTOR
                if data[at:at + size] == bytes(size):
                    continue
                offset, width = rng.choice(SWEEP_FIELDS[kind])
                value = struct.unpack_from("<" + width, data, at + offset)[0]
                value = value + rng.choice((-2, -1, 1, 2)) if rng.random() < 0.8 else rng.randrange(20)
                struct.pack_into("<" + width, data, at + offset, min(max(value, 0), LIMITS[width]))
                if kind == "record":
                    reseal_record(data, at)
                elif kind == "node":
                    reseal_node(data, at)
                else:
                    data[at + 508:at + 512] = bytes(4)
                    struct.pack_into("<I", data, at + 508, zlib.crc32(data[at:at + SECTOR]))
            reseal_header(data, generation)
            try:
                oracle7.snapshot(data)
                oracle_refusal = None
            except oracle7.Corrupt as error:
                oracle_refusal = str(error)
            with open(path, "r+b") as handle:
                handle.write(data[:prefix])
            _, rust_refusal = volume_tool("report7", path)
            if (oracle_refusal is None) != (rust_refusal is None):
                raise SystemExit(f"oracle7 and the Rust mount disagree on a {name} sweep case "
                                 f"(metadata sha256 {sha(bytes(data[:prefix]))}): oracle7={oracle_refusal!r} rust={rust_refusal!r}")
            totals["refused" if oracle_refusal else "accepted"] += 1
    shutil.rmtree(work)
    # A sweep that only ever refuses (or only accepts) would not compare the rules.
    floor = max(1, count * len(EXPORTS) // 10)
    if totals["accepted"] < floor or totals["refused"] < floor:
        raise SystemExit(f"differential sweep is one-sided: {totals}; each verdict needs at least {floor}")
    return {"seed": seed, "cases_per_image": count, "images": list(EXPORTS), **totals}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--sweep", type=int, default=60, help="resealed perturbations per exported image")
    parser.add_argument("--seed", type=int, default=51)
    arguments = parser.parse_args()
    output = arguments.output
    output.mkdir(parents=True, exist_ok=True)
    images = export(output)
    summary, history = verify_images(output, images)
    migrations = verify_migrations(output)
    damage = verify_damage(output, images / "v7-history.img", history)
    sweep = differential_sweep(output, images, arguments.sweep, arguments.seed)
    evidence = {
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=environment.ROOT).decode().strip(),
        "worktree_status": subprocess.check_output(["git", "status", "--porcelain"], cwd=environment.ROOT).decode(),
        "images": summary,
        "migrations": migrations,
        "damage": damage,
        "sweep": sweep,
        "exported": {name: {"bytes": (images / name).stat().st_size, "sha256": sha((images / name).read_bytes())}
                     for name in EXPORTS},
    }
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    refused = sum(1 for case in damage if case["expected"] == "refused")
    print(f"V7 images verified independently: {len(summary)} images agree with report7; "
          f"{len(migrations)} migrated v5 histories agree with the v5 reader and kept their source digest; "
          f"{len(damage)} damaged copies: {refused} refused and {len(damage) - refused} accepted "
          f"by both oracle7 and the Rust mount; {sweep['accepted'] + sweep['refused']} resealed perturbations "
          f"agree ({sweep['refused']} refused, {sweep['accepted']} accepted)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
