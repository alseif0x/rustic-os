# SPDX-License-Identifier: Apache-2.0
"""Owner retention maintenance of a fresh V7 volume in two disposable UEFI boots.

Each cycle fills the eight-record budget with 4 KiB tracked writes to the
seeded `scratch.bin`, checks that one more write is `Full`, has the owner run
`maintain-v7`, writes again in the new retry epoch and checks that the old
epoch has expired: an exact retry and a fresh write naming it are
`ExpiredEpoch`, a lookup by its retry key is `ExpiredEpoch` and a lookup of the
reclaimed operation ID is `OutcomeUnknown`. In the first cycle the owner first
asks for maintenance while an exact retry of the last write holds a transfer
open: the service answers `Busy`, the transfer is aborted, one more write is
still `Full` and the image is byte-identical. Boot 1 runs two cycles; boot 2
looks the last write up again, checks that the epoch persisted and runs a third.

The independent `oracle7` reader checks the image before and after every
maintenance while the guest is idle at the prompt (the shell prints only after
the service's publication and flushes, and QEMU writes through the host page
cache the reader sees), and again after each clean shutdown: the epoch advanced
by one, every record was dropped, exactly the sectors held only by retained
snapshots were freed and the live files did not change.
"""
import json
from pathlib import Path
import re
import tempfile
import time
import uuid

import environment
from . import oracle7
from .v7_read import volume_json
from .v7_write import (ERROR, LOOKUP_TIMING, RETAINED, SHELL_SUBJECT, boot_terminal, check_receipt, command,
                       decode, hex16, lookup_error, match_records, pattern)


ROOT = environment.ROOT
# Records `rustic-volume seed7 --scratch` provisions (ELF and manifest).
SEEDED_RECORDS = 2
# Eight sectors per write, so every superseded snapshot frees eight sectors.
SIZE = 4096
FIRST_KEY = 0x300
# Never committed: the fresh write that names an expired epoch.
STALE_KEY = 0x2FF
# The held retry sends 40 chunks (1,600 bytes, three verified sectors).
HOLD_CHUNKS = 40
CHUNK_BYTES = 40
BOOT_CYCLES = (2, 1)

MAINTAIN = re.compile(r"^maintain-v7 previous=e_([0-9a-f]{16}) epoch=e_([0-9a-f]{16}) records=(0|[1-9][0-9]{0,2}) "
                      r"sectors=(0|[1-9][0-9]{0,9}) job=([1-9][0-9]{0,15}) ticks=(0|[1-9][0-9]{0,15})$")
HOLD = re.compile(r"^hold-v7 chunks=([1-9][0-9]{0,6}) bytes=([1-9][0-9]{0,6}) "
                  r"maintain=([A-Za-z]+) abort=([A-Za-z]+)$")


def _single(text, prefixes, what):
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    lines = [line for line in lines if line.startswith(prefixes)]
    if len(lines) != 1:
        raise ValueError(f"incomplete or ambiguous {what} answer: {text!r}")
    return lines[0]


def decode_maintain(text):
    """Decode one `maintain-v7` answer: the report or one refusal."""
    # The echoed command is the bare verb; only the report carries fields.
    line = _single(text, ("maintain-v7 ", "error"), "maintenance")
    error = ERROR.match(line)
    if error:
        return {"error": error[1]}
    match = MAINTAIN.match(line)
    if not match:
        raise ValueError(f"malformed maintenance answer: {text!r}")
    previous, epoch = int(match[1], 16), int(match[2], 16)
    if epoch != previous + 1:
        raise ValueError("maintenance must advance the retry epoch by exactly one")
    return {"previous": previous, "epoch": epoch, "records": int(match[3]), "sectors": int(match[4]),
            "job": int(match[5]), "ticks": int(match[6])}


def decode_hold(text):
    """Decode one `replace-pattern-v7 ... hold N` answer: the observations or one error."""
    line = _single(text, ("operation-v1", "receipt", "write-v7", "maintain-v7 ", "hold-v7", "error"), "hold")
    error = ERROR.match(line)
    if error:
        return {"error": error[1]}
    match = HOLD.match(line)
    if not match:
        raise ValueError(f"malformed hold answer: {text!r}")
    return {"chunks": int(match[1]), "bytes": int(match[2]), "maintain": match[3], "abort": match[4]}


def reclaimable(snapshot):
    """Payload sectors held only by retained snapshots: exactly what maintenance frees."""
    return sum(count for record in snapshot["records"] if not record["aliases_live"]
               for _, count in record["runs"])


def check_maintained(before, after, maintained):
    """Oracle views around one maintenance must agree with the owner's report."""
    expected = {"previous": before["epoch"], "epoch": before["epoch"] + 1,
                "records": len(before["records"]), "sectors": reclaimable(before)}
    for field, value in expected.items():
        if maintained[field] != value:
            raise AssertionError(f"maintenance reported {field}={maintained[field]}, the image says {value}")
    if after["epoch"] != maintained["epoch"] or after["records"]:
        raise AssertionError("the published generation kept records or another epoch")
    if after["sequence"] != before["sequence"] + 1:
        raise AssertionError("maintenance did not publish exactly one generation")
    if after["free_sectors"] != before["free_sectors"] + maintained["sectors"]:
        raise AssertionError("free space did not grow by exactly the reclaimed snapshot sectors")
    if after["files"] != before["files"]:
        raise AssertionError("maintenance changed a live file")


def view(snapshot):
    """The fields of an oracle snapshot the evidence records."""
    return {"epoch": snapshot["epoch"], "sequence": snapshot["sequence"], "records": len(snapshot["records"]),
            "free_sectors": snapshot["free_sectors"]}


def _live(data):
    """Oracle snapshot while the guest is idle at the prompt."""
    return oracle7.snapshot(data.read_bytes())


def _write(uart, refs, version, epoch, key, seed, size):
    started = time.monotonic()
    result = decode(uart.command(command(refs["workspace"], refs["resource"], version, epoch, key, seed, size), "\n"))
    result["host_seconds"] = round(time.monotonic() - started, 3)
    return result


def _commit(uart, refs, lineage, state, seed):
    """One fresh write of the next key in the current epoch; it must succeed."""
    key = state["key"]
    receipt = _write(uart, refs, state["version"], state["epoch"], key, seed, SIZE)
    if "error" in receipt:
        raise AssertionError(f"write of key {key:#x} in epoch {state['epoch']} failed: {receipt['error']}")
    check_receipt(receipt, lineage, refs["workspace"], refs["resource"], state["version"], state["epoch"], key,
                  pattern(seed, SIZE))
    state["key"] += 1
    state["version"] = receipt["version"]
    return {**receipt, "seed": seed}


def _full(uart, refs, state):
    refused = _write(uart, refs, state["version"], state["epoch"], state["key"], 9, 64)
    if refused.get("error") != "Full":
        raise AssertionError(f"write beyond the retained budget was not Full: {refused}")
    return "Full"


def _hold(uart, data, refs, state, last):
    """Maintenance while an exact retry of `last` holds a transfer: Busy, nothing changed."""
    before = environment.digest(data)
    text = uart.command(command(refs["workspace"], refs["resource"], last["previous"], last["epoch"], last["key"],
                                last["seed"], SIZE) + f" hold {HOLD_CHUNKS}", "\n")
    hold = decode_hold(text)
    expected = {"chunks": HOLD_CHUNKS, "bytes": HOLD_CHUNKS * CHUNK_BYTES, "maintain": "Busy", "abort": "ok"}
    if "error" in hold or any(hold[field] != value for field, value in expected.items()):
        raise AssertionError(f"maintenance during an open transfer was not refused as expected: {hold}")
    hold["after"] = _full(uart, refs, state)
    if environment.digest(data) != before:
        raise AssertionError("the refused maintenance or the aborted retry changed the image")
    hold["image_unchanged"] = True
    hold["retry_key"] = last["key"]
    return hold


def _expired(uart, refs, state, old):
    """Every way of naming the reclaimed epoch after maintenance."""
    answers = {}
    retry = _write(uart, refs, old["previous"], old["epoch"], old["key"], old["seed"], old["size"])
    answers["exact_retry"] = retry.get("error")
    fresh = _write(uart, refs, state["version"], old["epoch"], STALE_KEY, 9, 64)
    answers["fresh_write"] = fresh.get("error")
    answers["lookup_retry"] = lookup_error(uart.command(
        f"operation-v7 {refs['workspace']} e_{hex16(old['epoch'])} k_{hex16(old['key'])}", "error:"))
    answers["lookup_id"] = lookup_error(uart.command(f"operation-v7 {old['id']}", "error:"))
    expected = {"exact_retry": "ExpiredEpoch", "fresh_write": "ExpiredEpoch", "lookup_retry": "ExpiredEpoch",
                "lookup_id": "OutcomeUnknown"}
    if answers != expected:
        raise AssertionError(f"old-epoch answers after maintenance: {answers}, expected {expected}")
    return answers


def _cycle(uart, data, refs, lineage, state, number, hold):
    """Fill to Full, maintain (after a refused attempt when `hold`), write again."""
    free = RETAINED - len(_live(data)["records"])
    writes = [_commit(uart, refs, lineage, state, (number * 16 + index + 1) & 0xFF) for index in range(free)]
    full = _full(uart, refs, state)
    before = _live(data)
    if len(before["records"]) != RETAINED:
        raise AssertionError(f"cycle {number} did not fill the retained budget")
    held = _hold(uart, data, refs, state, writes[-1]) if hold else None
    started = time.monotonic()
    maintained = decode_maintain(uart.command("maintain-v7", "\n"))
    if "error" in maintained:
        raise AssertionError(f"maintenance in cycle {number} failed: {maintained['error']}")
    maintained["host_seconds"] = round(time.monotonic() - started, 3)
    after = _live(data)
    check_maintained(before, after, maintained)
    state["epoch"] = maintained["epoch"]
    useful = _commit(uart, refs, lineage, state, (number * 16 + 15) & 0xFF)
    if useful["previous"] != writes[-1]["version"]:
        raise AssertionError("the write after maintenance does not follow the last write")
    expired = _expired(uart, refs, state, writes[0])
    return {"cycle": number, "writes": writes, "beyond_budget": full, "hold": held, "maintenance": maintained,
            "before": view(before), "after": view(after), "useful": useful, "expired": expired}


def _persisted(uart, refs, state, last, previous_epoch_key):
    """After a reboot the last write is found in the persisted epoch; the one before has expired."""
    found = []
    for name, query in (("id", last["id"]),
                        ("retry", f"{refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(last['key'])}")):
        result = decode(uart.command(f"operation-v7 {query}", "\n"), LOOKUP_TIMING)
        if "error" in result or result["lines"] != last["lines"]:
            raise AssertionError(f"lookup by {name} after reboot differs: {result}")
        found.append({"by": name, "ticks": result["ticks"], "identical_receipt": True})
    epoch, key = previous_epoch_key
    expired = lookup_error(uart.command(f"operation-v7 {refs['workspace']} e_{hex16(epoch)} k_{hex16(key)}",
                                        "error:"))
    if expired != "ExpiredEpoch":
        raise AssertionError(f"a key of the previous epoch was not expired after reboot: {expired}")
    return {"lookups": found, "previous_epoch_lookup": expired}


def _offline(data, state, last, seeded, scratch, sources):
    """The image after a clean shutdown: persisted epoch, one record, live file and pair."""
    snapshot = oracle7.snapshot(data.read_bytes())
    if snapshot["epoch"] != state["epoch"] or len(snapshot["records"]) != 1:
        raise AssertionError(f"the persisted epoch or records differ: {view(snapshot)}")
    match_records(snapshot, [last], seeded["workspace"]["id"], scratch["id"])
    live = next(item for item in snapshot["files"] if item["id"] == scratch["id"])
    if (live["version"], live["size"], live["sha256"]) != (last["version"], last["size"], last["sha256"]):
        raise AssertionError("the live scratch file is not the last committed pattern")
    for source in sources:
        if snapshot["contents"].get(f"/workspaces/application/{source.name}") != source.read_bytes():
            raise AssertionError(f"the retention cycles changed {source.name}")
    return view(snapshot)


def _compact(write):
    return {key: write[key] for key in ("key", "seed", "previous", "version", "epoch", "size", "sha256", "ticks")}


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-retention")
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
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-retention-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-volume.raw"
            lineage = uuid.uuid4().hex
            seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
            scratch = seeded["scratch"]
            refs = {"workspace": seeded["workspace"]["text"], "resource": scratch["resource"]}
            initial = oracle7.snapshot(data.read_bytes())
            if len(initial["records"]) != SEEDED_RECORDS:
                raise AssertionError(f"seed7 provisioned {len(initial['records'])} records, expected {SEEDED_RECORDS}")
            state = {"epoch": initial["epoch"], "version": scratch["version"], "key": FIRST_KEY}
            cycles = []

            def first_boot(uart):
                for number in range(BOOT_CYCLES[0]):
                    cycles.append(_cycle(uart, data, refs, lineage, state, number, hold=number == 0))

            boot_terminal(image, data, output, 1, temporary, first_boot)
            after_first = _offline(data, state, cycles[-1]["useful"], seeded, scratch, (elf, manifest))
            report = volume_json(volume_tool, "report7", data)
            if report["sequence"] != after_first["sequence"] or len(report["records"]) != 1:
                raise AssertionError("report7 and oracle7 disagree after boot 1")

            persisted = {}

            def second_boot(uart):
                last = cycles[-1]["useful"]
                older = cycles[-1]["writes"][0]
                persisted.update(_persisted(uart, refs, state, last, (older["epoch"], older["key"])))
                for number in range(BOOT_CYCLES[0], sum(BOOT_CYCLES)):
                    cycles.append(_cycle(uart, data, refs, lineage, state, number, hold=False))

            boot_terminal(image, data, output, 2, temporary, second_boot)
            after_second = _offline(data, state, cycles[-1]["useful"], seeded, scratch, (elf, manifest))

        maintenances = [{"cycle": item["cycle"], **{key: item["maintenance"][key] for key in
                                                    ("previous", "epoch", "records", "sectors", "ticks",
                                                     "host_seconds")},
                         "guest_seconds": item["maintenance"]["ticks"] / 100} for item in cycles]
        evidence = {
            "verified": True,
            "mode": "terminal-v7",
            "boots": 2,
            "lineage": lineage,
            "subject": SHELL_SUBJECT,
            "scratch": scratch,
            "initial": view(initial),
            "cycles": [{"cycle": item["cycle"], "boot": 1 if item["cycle"] < BOOT_CYCLES[0] else 2,
                        "writes": [_compact(write) for write in item["writes"]],
                        "beyond_budget": item["beyond_budget"], "hold": item["hold"],
                        "maintenance": item["maintenance"], "oracle_before": item["before"],
                        "oracle_after": item["after"], "useful_write": _compact(item["useful"]),
                        "old_epoch": item["expired"]} for item in cycles],
            "maintenances": maintenances,
            "after_boot_1": after_first,
            "after_boot_2": {**after_second, **persisted},
        }
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "terminal_v7_retention": evidence,
        }
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print(f"V7 retention acceptance: {len(cycles)} fill/maintain/write cycles across two boots; maintenance "
              "Busy while an exact retry held a transfer (image unchanged, still Full), then each maintenance "
              "advanced the epoch by one, dropped all eight records and freed exactly the snapshot-only sectors "
              "(oracle7 before and after); writes in each new epoch succeeded; old-epoch retries, fresh writes and "
              "retry lookups ExpiredEpoch, reclaimed IDs OutcomeUnknown; epoch and record persisted across reboot.",
              flush=True)
        for item in maintenances:
            print(f"V7 maintenance: cycle={item['cycle']} epoch={item['previous']}->{item['epoch']} "
                  f"records={item['records']} sectors={item['sectors']} guest_ticks={item['ticks']} "
                  f"host_seconds={item['host_seconds']}", flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(
            b"\n".join((output / f"serial-{p}.log").read_bytes() for p in phases if (output / f"serial-{p}.log").exists()))
        (output / "qemu.log").write_bytes(
            b"\n".join((output / f"qemu-{p}.log").read_bytes() for p in phases if (output / f"qemu-{p}.log").exists()))
