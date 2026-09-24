# SPDX-License-Identifier: Apache-2.0
"""Storage exhaustion of a nearly full V7 volume in two disposable UEFI boots.

The host builds the volume: `rustic-volume seed7 --scratch`, then small files
and 512 KiB files published one by one with `add7` (with `maintain7` whenever
the eight retained-record slots are held, since every `add7` retains one)
until the shared payload region has exactly `FREE_TARGET` free sectors, and a
final `maintain7` so the guest starts with every record slot free. The volume
then holds far more than the old 32-object limit, and the eight largest free
runs can hold `FREE_TARGET` sectors, so a write is refused for storage alone.

Boot 1 changes nothing: it times a full remount (`restart files timed`),
reads object 48 and the last small file completely and the highest object's
first KiB by reference, then asks for a 64 KiB and a 512 KiB tracked write and
the 64 KiB write again. Each is `Full`; the repeat is `Full`, not `Busy`, so
the refused open left no transfer on the slot. The image digest and the
`oracle7` view are unchanged by the boot.

Boot 2 commits two 8 KiB writes (the first one's sectors are then held only by
its retained snapshot), is refused a 76-sector write for storage, has the
owner run `maintain-v7` (it must free exactly the snapshot-only sectors, as
`oracle7` counts them before and after while the guest is idle), and then
commits the same 76-sector write in the new epoch. `commit_ticks` of every
commit and the maintenance ticks are recorded as the blocking-section bounds.

A separate host step copies the pre-boot image and fills the 256-entry object
table with `add7`: the next `add7` is refused. The V7 file service has no
create request, so object-table exhaustion cannot be reached from the guest.
"""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import uuid
import base64

import environment
from . import oracle7
from .read_cases import read as read_range
from .v7_read import volume_json
from .v7_retention import check_maintained, decode_maintain, reclaimable, view
from .v7_write import ERROR, boot_terminal, check_receipt, command, decode, hex16, match_records, pattern


ROOT = environment.ROOT
# Payload region of format 7 (docs/WORKSPACE-FORMAT7.md) and its object table.
PAYLOAD_SECTORS = 131_072
OBJECTS = 256
SECTOR = 512
BIG_BYTES = 512 * 1024
RETAINED = 8
# Small files are published first, so their object IDs start right after the
# seed's eight nodes; each fits one 1 KiB read.
SEED_NODES = 8
SMALL_FILES = 64
FIRST_SMALL_ID = SEED_NODES + 1
# Object 48 is well above the old 32-object limit.
READ_IDS = (48, FIRST_SMALL_ID + SMALL_FILES - 1)
FREE_TARGET = 100
MIN_OBJECTS = 150
# Tracked writes to `scratch.bin`. Sizes are chosen against FREE_TARGET.
WRITE_SIZE = 8 * 1024
REFUSED_SIZES = (64 * 1024, 512 * 1024)
# 76 sectors: more than remain after two 8 KiB writes (68), fewer than remain
# once maintenance frees the first write's 16 snapshot-only sectors (84).
RECLAIM_SIZE = 76 * SECTOR
FIRST_KEY = 0x500
REFUSED_KEY = 0x5F0
READ_BYTES = 1024

RESTART = re.compile(r"^restart-files ticks=(0|[1-9][0-9]{0,15})$")
# A refused restart prints one shell error, whose text is not a file error name.
RESTART_ERROR = re.compile(r"^error: (\S.*)$")


def small_content(index):
    """Deterministic bytes of small file `index`: 200..1019 bytes, distinct seeds."""
    return pattern(index + 1, 200 + 13 * index)


def big_content():
    return pattern(0xB0, BIG_BYTES)


def sectors(size):
    return -(-size // SECTOR)


def plan_big(free, target=FREE_TARGET):
    """Split `free - target` sectors into whole 512 KiB files plus one tail file size in bytes."""
    remaining = free - target
    if remaining < 0:
        raise ValueError("the volume already has fewer free sectors than the target")
    full, tail = divmod(remaining, BIG_BYTES // SECTOR)
    return full, tail * SECTOR


def decode_restart(text):
    """Decode one `restart files timed` answer: the restart's guest ticks or one refusal."""
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    lines = [line for line in lines if line.startswith(("restart-files", "files restarted", "error"))]
    errors = [RESTART_ERROR.match(line) for line in lines if line.startswith("error")]
    if errors:
        if len(lines) != 1 or not errors[0]:
            raise ValueError(f"ambiguous restart failure: {text!r}")
        return {"error": errors[0][1]}
    timings = [RESTART.match(line) for line in lines if line.startswith("restart-files")]
    if lines.count("files restarted; utility sessions revoked") != 1 or len(timings) != 1 or not timings[0]:
        raise ValueError(f"incomplete or ambiguous restart answer: {text!r}")
    return {"ticks": int(timings[0][1])}


def _publish(tool, image, name, data, temporary, state):
    """`add7` one file into `/workspaces/application`, retiring records first when all are held."""
    if state["records"] == RETAINED:
        volume_json(tool, "maintain7", image)
        state["records"] = 0
        state["maintenances"] += 1
    source = temporary / "source.bin"
    source.write_bytes(data)
    added = volume_json(tool, "add7", image, "/workspaces/application", name, source)
    state["records"] += 1
    return added["file"]


def fill(tool, image, lineage, elf, manifest, temporary):
    """Build the nearly full volume and return what the guest checks."""
    seeded = volume_json(tool, "seed7", image, lineage, elf, manifest, "--scratch")
    state = {"records": len(oracle7.snapshot(image.read_bytes())["records"]), "maintenances": 0}
    small = []
    for index in range(SMALL_FILES):
        added = _publish(tool, image, f"s{index:03}.bin", small_content(index), temporary, state)
        if added["id"] != FIRST_SMALL_ID + index:
            raise AssertionError(f"small file {index} got object {added['id']}")
        small.append(added)
    full, tail = plan_big(oracle7.snapshot(image.read_bytes())["free_sectors"])
    big = big_content()
    last = None
    for index in range(full):
        last = _publish(tool, image, f"b{index:03}.bin", big, temporary, state)
    if tail:
        last = _publish(tool, image, "tail.bin", big[:tail], temporary, state)
    final = volume_json(tool, "maintain7", image)
    state["maintenances"] += 1
    snapshot = oracle7.snapshot(image.read_bytes())
    if snapshot["free_sectors"] != FREE_TARGET or snapshot["records"]:
        raise AssertionError(f"fill left {snapshot['free_sectors']} free sectors and {len(snapshot['records'])} records")
    if len(snapshot["nodes"]) < MIN_OBJECTS:
        raise AssertionError(f"the volume holds only {len(snapshot['nodes'])} objects")
    if snapshot["used_sectors"] + snapshot["free_sectors"] != PAYLOAD_SECTORS:
        raise AssertionError("oracle7 used and free sectors do not cover the payload region")
    reads = [{"id": item["id"], "resource": item["resource"], "version": item["version"],
              "content": small_content(item["id"] - FIRST_SMALL_ID)}
             for item in small if item["id"] in READ_IDS]
    reads.append({"id": last["id"], "resource": last["resource"], "version": last["version"],
                  "content": big[:last["size"]][:READ_BYTES], "size": last["size"]})
    return {
        "seeded": seeded, "reads": reads, "big_files": full, "tail_bytes": tail,
        "host_maintenances": state["maintenances"], "epoch": final["epoch"],
    }


def exhaust_objects(tool, image, temporary):
    """Fill the object table of a copy with one-sector files until `add7` is refused."""
    copy = temporary / "objects.raw"
    shutil.copyfile(image, copy)
    state = {"records": 0, "maintenances": 0}
    snapshot = oracle7.snapshot(copy.read_bytes())
    added = 0
    for index in range(OBJECTS - len(snapshot["nodes"])):
        _publish(tool, copy, f"o{index:03}.bin", bytes([index & 0xFF]) * 16, temporary, state)
        added += 1
    full = oracle7.snapshot(copy.read_bytes())
    if len(full["nodes"]) != OBJECTS:
        raise AssertionError(f"the object table holds {len(full['nodes'])} entries, not {OBJECTS}")
    if state["records"] == RETAINED:
        volume_json(tool, "maintain7", copy)
    before = environment.digest(copy)
    source = temporary / "source.bin"
    source.write_bytes(b"x")
    result = subprocess.run([str(tool), "add7", str(copy), "/workspaces/application", "one-more.bin", str(source)],
                            capture_output=True, text=True)
    if result.returncode == 0:
        raise AssertionError("add7 created an object beyond the 256-entry table")
    if environment.digest(copy) != before:
        raise AssertionError("the refused add7 changed the image")
    return {"objects": len(full["nodes"]), "added": added, "free_sectors": full["free_sectors"],
            "refusal": result.stderr.strip(), "image_unchanged": True}


def _refuse(uart, refs, version, epoch, key, seed, size):
    started = time.monotonic()
    answer = decode(uart.command(command(refs["workspace"], refs["resource"], version, epoch, key, seed, size), "\n"))
    if answer != {"error": "Full"}:
        raise AssertionError(f"a {size}-byte write was not refused with Full: {answer}")
    return {"size": size, "sectors": sectors(size), "status": "Full",
            "host_seconds": round(time.monotonic() - started, 3)}


def _commit(uart, refs, lineage, version, epoch, key, seed, size):
    started = time.monotonic()
    receipt = decode(uart.command(command(refs["workspace"], refs["resource"], version, epoch, key, seed, size), "\n"))
    if "error" in receipt:
        raise AssertionError(f"a {size}-byte write failed: {receipt['error']}")
    if "commit_ticks" not in receipt:
        raise AssertionError("the write did not report its commit ticks")
    check_receipt(receipt, lineage, refs["workspace"], refs["resource"], version, epoch, key, pattern(seed, size))
    receipt["host_seconds"] = round(time.monotonic() - started, 3)
    receipt["seed"] = seed
    return receipt


def _read(uart, workspace, item):
    refs = {"workspace": workspace, "resource": item["resource"]}
    result = read_range(uart, refs, 0, READ_BYTES)["result"]
    data = base64.b64decode(result["data"])
    size = item.get("size", len(item["content"]))
    if data != item["content"] or result["size"] != size or result["version"] != f"v_{hex16(item['version'])}":
        raise AssertionError(f"guest read of object {item['id']} does not match the host bytes")
    return {"id": item["id"], "bytes": len(data), "size": size, "sha256": hashlib.sha256(data).hexdigest()}


def _first_boot(uart, refs, workspace, reads, epoch, version):
    restart = decode_restart(uart.command("restart files timed", "\n"))
    if "error" in restart:
        raise AssertionError(f"remount of the full volume failed: {restart['error']}")
    observed = [_read(uart, workspace, item) for item in reads]
    refused = [_refuse(uart, refs, version, epoch, REFUSED_KEY + index, 30 + index, size)
               for index, size in enumerate(REFUSED_SIZES)]
    # Busy here would mean the first refusal left a transfer on the slot.
    refused.append(_refuse(uart, refs, version, epoch, REFUSED_KEY, 30, REFUSED_SIZES[0]))
    if restart["ticks"] <= 0:
        raise AssertionError("a nearly full volume cannot mount in zero ticks; the measurement is broken")
    return {"mount_ticks": restart["ticks"], "reads": observed, "refusals": refused}


def _second_boot(uart, data, refs, lineage, epoch, version):
    writes = []
    for index in range(2):
        receipt = _commit(uart, refs, lineage, version, epoch, FIRST_KEY + index, index + 1, WRITE_SIZE)
        writes.append(receipt)
        version = receipt["version"]
    before = oracle7.snapshot(data.read_bytes())
    refused = _refuse(uart, refs, version, epoch, FIRST_KEY + 2, 3, RECLAIM_SIZE)
    refused["free_sectors"] = before["free_sectors"]
    maintained = decode_maintain(uart.command("maintain-v7", "\n"))
    if "error" in maintained:
        raise AssertionError(f"maintenance was refused: {maintained['error']}")
    after = oracle7.snapshot(data.read_bytes())
    check_maintained(before, after, maintained)
    if maintained["sectors"] != sectors(WRITE_SIZE):
        raise AssertionError(f"maintenance freed {maintained['sectors']} sectors, not the first write's snapshot")
    reclaimed = _commit(uart, refs, lineage, version, maintained["epoch"], FIRST_KEY + 2, 3, RECLAIM_SIZE)
    return {"writes": writes, "refused": refused, "maintenance": maintained, "reclaimed": reclaimed,
            "oracle": {"before_maintenance": view(before), "after_maintenance": view(after),
                       "snapshot_only_sectors": reclaimable(before)}}


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-capacity")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    for path, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(path) != metadata["native_applications"]["file-server"][suffix]:
            raise RuntimeError(f"{path.name} differs from the artifact recorded by the boot build")
    (output / "result.json").unlink(missing_ok=True)
    started = time.monotonic()
    phases = [1, 2]
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-capacity-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-volume.raw"
            lineage = uuid.uuid4().hex
            fill_started = time.monotonic()
            built = fill(volume_tool, data, lineage, elf, manifest, temporary)
            fill_seconds = round(time.monotonic() - fill_started, 3)
            objects = exhaust_objects(volume_tool, data, temporary)
            seeded = built["seeded"]
            scratch = seeded["scratch"]
            workspace = seeded["workspace"]["text"]
            refs = {"workspace": workspace, "resource": scratch["resource"]}
            initial = oracle7.snapshot(data.read_bytes())
            epoch = initial["epoch"]
            if epoch != built["epoch"]:
                raise AssertionError("oracle7 and maintain7 disagree on the epoch")
            before_first = environment.digest(data)

            first = boot_terminal(image, data, output, 1, temporary,
                                  lambda uart: _first_boot(uart, refs, workspace, built["reads"], epoch,
                                                           scratch["version"]))
            after_first = oracle7.snapshot(data.read_bytes())
            if environment.digest(data) != before_first:
                raise AssertionError("boot 1 (reads and refused writes) changed the V7 volume")
            if view(after_first) != view(initial) or after_first["files"] != initial["files"]:
                raise AssertionError("oracle7 sees a different volume after boot 1")

            second = boot_terminal(image, data, output, 2, temporary,
                                   lambda uart: _second_boot(uart, data, refs, lineage, epoch, scratch["version"]))
            final = oracle7.snapshot(data.read_bytes())
            reclaimed = second["reclaimed"]
            match_records(final, [reclaimed], seeded["workspace"]["id"], scratch["id"])
            if len(final["records"]) != 1:
                raise AssertionError("the final generation holds records beyond the reclaimed write")
            live = next(item for item in final["files"] if item["id"] == scratch["id"])
            if (live["version"], live["size"], live["sha256"]) != (reclaimed["version"], reclaimed["size"],
                                                                     reclaimed["sha256"]):
                raise AssertionError("the live scratch file is not the last committed pattern")
            # The superseded 8 KiB payload was released at commit (its record
            # was dropped by maintenance); the new one took 76 sectors.
            expected_free = (second["oracle"]["after_maintenance"]["free_sectors"]
                             - sectors(RECLAIM_SIZE) + sectors(WRITE_SIZE))
            if final["free_sectors"] != expected_free:
                raise AssertionError(f"final free sectors {final['free_sectors']}, expected {expected_free}")
            others = [item for item in final["files"] if item["id"] != scratch["id"]]
            if others != [item for item in initial["files"] if item["id"] != scratch["id"]]:
                raise AssertionError("a file other than scratch.bin changed")

        commits = [{"size": item["size"], "commit_ticks": item["commit_ticks"], "write_ticks": item["ticks"],
                    "host_seconds": item["host_seconds"]} for item in (*second["writes"], reclaimed)]
        evidence = {
            "verified": True,
            "mode": "terminal-v7",
            "boots": 2,
            "lineage": lineage,
            "volume": {
                "objects": len(initial["nodes"]),
                "files": len(initial["files"]),
                "small_files": SMALL_FILES,
                "big_files": built["big_files"],
                "tail_bytes": built["tail_bytes"],
                "payload_sectors": PAYLOAD_SECTORS,
                "used_sectors": initial["used_sectors"],
                "free_sectors": initial["free_sectors"],
                "records_free": RETAINED - len(initial["records"]),
                "host_maintenances": built["host_maintenances"],
                "fill_seconds": fill_seconds,
                "sha256_before_boot_1": before_first,
            },
            "mount_ticks": first["mount_ticks"],
            "reads": first["reads"],
            "refusals": first["refusals"] + [second["refused"]],
            "boot_1_unchanged": True,
            "writes": [{key: item[key] for key in ("id", "previous", "version", "size", "epoch", "key", "seed",
                                                    "sha256")}
                       for item in (*second["writes"], reclaimed)],
            "commits": commits,
            "maintenance": {key: second["maintenance"][key] for key in ("previous", "epoch", "records", "sectors",
                                                                       "ticks")},
            "oracle": {**second["oracle"], "final": view(final)},
            "object_table": objects,
        }
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "terminal_v7_capacity": evidence,
        }
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        volume = evidence["volume"]
        print(f"V7 capacity acceptance: {volume['objects']} objects, {volume['used_sectors']} of "
              f"{PAYLOAD_SECTORS} payload sectors used ({volume['free_sectors']} free), every record slot free; "
              f"objects {', '.join(str(item['id']) for item in first['reads'])} read by reference; "
              f"64 KiB and 512 KiB writes Full with no transfer left open and the image unchanged; two 8 KiB "
              f"writes committed; a {RECLAIM_SIZE}-byte write Full, then committed after maintenance freed "
              f"{second['maintenance']['sectors']} snapshot-only sectors; the object table filled to "
              f"{objects['objects']} on a host copy and add7 refused beyond it.", flush=True)
        print(f"V7 capacity: mount_ticks={first['mount_ticks']} maintenance_ticks={second['maintenance']['ticks']}",
              flush=True)
        for item in commits:
            print(f"V7 capacity commit: size={item['size']} commit_ticks={item['commit_ticks']} "
                  f"write_ticks={item['write_ticks']}", flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(
            b"\n".join((output / f"serial-{p}.log").read_bytes() for p in phases if (output / f"serial-{p}.log").exists()))
        (output / "qemu.log").write_bytes(
            b"\n".join((output / f"qemu-{p}.log").read_bytes() for p in phases if (output / f"qemu-{p}.log").exists()))
