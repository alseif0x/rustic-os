# SPDX-License-Identifier: Apache-2.0
"""Interrupted V7 publication in the guest, bounded memory and control latency (#51).

The V7 file server uses blocking copied-sector I/O, so the device sees one
event per call in program order. A fresh tracked write of `n` sectors is `n`
payload writes, then the publication: the inactive generation's node, map and
receipt sectors, a flush, the header write and a final flush. Retention
maintenance is the same publication without payload. A mount only reads and
flushes, and an exact retry only reads.

Each cut boots a copy of a base image with one blkdebug EIO armed at that event
(`recovery_faults.rules`): the event and everything later never reach the
image, everything earlier is in the host page cache the independent reader
sees. The operation must report `Uncertain`; `restart files` must mount the
generation the image holds, which `oracle7` decides while the guest is idle.
Only a cut at the final flush leaves the new header in the image, so only that
cut ends in the new generation. The guest's lookups, reads and a clean reboot
must agree with the oracle, and an exact retry (or a repeated maintenance)
must replay or commit consistently.

A budget boot records system memory at idle, during 512 KiB writes (through
the shell's `probe` diagnostic, which also times owner `INFO` round trips
between chunks), after each commit and after an owner revocation, whose
latency `cut-v7` reports. Fault boots record memory before the operation,
while the volume is fenced and after the restart; reboots record it at idle.
The static bound of the file server is its PT_LOAD pages plus the loader's
stack pages.
"""
import contextlib
import hashlib
import json
from pathlib import Path
import re
import struct
import tempfile
import time
import uuid

import environment
from . import oracle7
from .connection import Connection
from .machine import machine
from .read_cases import read as read_range
from .recovery_faults import rules
from .v7_read import volume_json
from .v7_retention import check_maintained, decode_maintain, reclaimable
from .v7_write import (LOOKUP_TIMING, SHELL_SUBJECT, check_receipt, command, decode, decode_cut, hex16,
                       lookup_error, match_records, pattern)


ROOT = environment.ROOT
BOOT_TIMEOUT = 300
SECTOR = oracle7.SECTOR
# The loader maps this many stack pages next to the PT_LOAD pages
# (kernel/src/process/elf.rs STACK_PAGES); the image budget is MAX_PAGES.
STACK_PAGES = 16
IMAGE_PAGE_BUDGET = 256
PAGE = 4096

# The interrupted write: 8 KiB, sixteen payload sectors.
WRITE_SIZE = 8 * 1024
WRITE_KEY = 0x500
WRITE_SEED = 5
# The budget boot's writes; the second and third supersede the first two.
LARGE_SIZE = 512 * 1024
PROBE_EVERY = 64
BUDGET_KEY = 0x400
BUDGET_SEEDS = (11, 12)
SMALL_SIZE = 4096
SMALL_SEED = 13
CUT_KEY = 0x4FF
CUT_CHUNKS = 400
CHUNK_BYTES = 40

MEM = re.compile(r"^ticks=(0|[1-9][0-9]*) free_frames=(0|[1-9][0-9]*) process_slots=(0|[1-9][0-9]*) "
                 r"processes=(0|[1-9][0-9]*) channels=(0|[1-9][0-9]*) pending_io=(0|[1-9][0-9]*) "
                 r"heap_pages=(0|[1-9][0-9]*)$")
PROBE = re.compile(r"^probe-v7 every=([1-9][0-9]*) probes=(0|[1-9][0-9]*) max=(0|[1-9][0-9]*) "
                   r"p50=(0|[1-9][0-9]*) total=(0|[1-9][0-9]*) free_min=(0|[1-9][0-9]*) "
                   r"free_max=(0|[1-9][0-9]*) heap_min=(0|[1-9][0-9]*) heap_max=(0|[1-9][0-9]*)$")
MEM_FIELDS = ("ticks", "free_frames", "process_slots", "processes", "channels", "pending_io", "heap_pages")
PROBE_FIELDS = ("every", "probes", "max", "p50", "total", "free_min", "free_max", "heap_min", "heap_max")


def publication_events():
    """Device events of one V7 publication: generation sectors, flush, header, flush."""
    metadata = oracle7.NODES_SECTORS + oracle7.MAP_SECTORS + oracle7.RECEIPTS_SECTORS
    return ["write_aio"] * metadata + ["flush_to_disk", "write_aio", "flush_to_disk"]


def write_events(size):
    """Device events of a fresh tracked write of `size` bytes: payload, then publication."""
    return ["write_aio"] * -(-size // SECTOR) + publication_events()


def _publication_cuts(first):
    nodes, maps, receipts = oracle7.NODES_SECTORS, oracle7.MAP_SECTORS, oracle7.RECEIPTS_SECTORS
    return {
        "nodes_first": first, "nodes_last": first + nodes - 1,
        "map_first": first + nodes, "map_last": first + nodes + maps - 1,
        "receipts_first": first + nodes + maps, "receipts_last": first + nodes + maps + receipts - 1,
        "metadata_flush": first + nodes + maps + receipts,
        "header": first + nodes + maps + receipts + 1,
        "final_flush": first + nodes + maps + receipts + 2,
    }


def write_cuts(size):
    """Named cut indexes of a fresh tracked write: first, second and last payload sector, then each boundary."""
    sectors = -(-size // SECTOR)
    if sectors < 3:
        raise ValueError("the write cut plan needs at least three payload sectors")
    return {"payload_first": 0, "payload_second": 1, "payload_last": sectors - 1, **_publication_cuts(sectors)}


def maintenance_cuts():
    """Named cut indexes of retention maintenance: the publication alone."""
    return _publication_cuts(0)


def adopts_new_generation(events, cut):
    """Whether a fail-stop before event `cut` leaves the new header in the image."""
    header = len(events) - 2
    if events[header] != "write_aio" or events[-1] != "flush_to_disk":
        raise ValueError("a publication ends with the header write and a flush")
    return cut > header


def decode_mem(text):
    """The single `mem` line."""
    lines = [MEM.match(line.strip()) for line in text.replace("\r\n", "\n").split("\n")]
    lines = [match for match in lines if match]
    if len(lines) != 1:
        raise ValueError(f"incomplete or ambiguous mem answer: {text!r}")
    return {field: int(value) for field, value in zip(MEM_FIELDS, lines[0].groups())}


def decode_probe(text):
    """A committed `replace-pattern-v7 ... probe N` answer: the receipt plus the probe line."""
    receipt = decode(text)
    if "error" in receipt:
        return receipt
    lines = [PROBE.match(line.strip()) for line in text.replace("\r\n", "\n").split("\n")
             if line.strip().startswith("probe-v7")]
    if len(lines) != 1 or not lines[0]:
        raise ValueError(f"incomplete or ambiguous probe answer: {text!r}")
    probe = {field: int(value) for field, value in zip(PROBE_FIELDS, lines[0].groups())}
    if probe["probes"] == 0 or not probe["p50"] <= probe["max"] or probe["total"] < probe["max"]:
        raise ValueError(f"inconsistent probe statistics: {text!r}")
    if probe["free_min"] > probe["free_max"] or probe["heap_min"] > probe["heap_max"]:
        raise ValueError(f"inconsistent probe memory range: {text!r}")
    return {**receipt, "probe": probe}


def expected_probes(size, every):
    """Samples the probe takes: before chunk k for every k < chunks with k % every == 0, then before the commit."""
    chunks = -(-size // CHUNK_BYTES)
    return -(-chunks // every) + 1


def static_pages(elf):
    """Pages the loader maps for an ELF64 image: its PT_LOAD pages plus the stack."""
    if elf[:6] != b"\x7fELF\x02\x01":
        raise ValueError("not a little-endian ELF64 image")
    offset, = struct.unpack_from("<Q", elf, 32)
    size, count = struct.unpack_from("<HH", elf, 54)
    if size != 56:
        raise ValueError("unexpected program header size")
    segments = []
    for index in range(count):
        kind, flags, _, address, _, file_size, memory_size = struct.unpack_from("<IIQQQQQ", elf, offset + index * 56)
        if kind != 1:
            continue
        start, end = address // PAGE, -(-(address + memory_size) // PAGE)
        segments.append({"address": address, "file_size": file_size, "memory_size": memory_size,
                         "writable": bool(flags & 2), "executable": bool(flags & 1), "pages": end - start})
    load = sum(segment["pages"] for segment in segments)
    return {"segments": segments, "load_pages": load, "stack_pages": STACK_PAGES,
            "pages": load + STACK_PAGES, "bytes": (load + STACK_PAGES) * PAGE,
            "image_page_budget": IMAGE_PAGE_BUDGET}


@contextlib.contextmanager
def _session(image, data, output, name, temporary, fault_rules=None):
    """One boot over `data`; the body runs between the mount check and a clean exit."""
    sock = temporary / f"{name}.sock"
    with machine(image, data, f"unix:{sock},server=on,wait=off", output / f"{name}.qemu.log",
                 rules=fault_rules) as vm:
        uart = Connection(sock, vm, output / f"{name}.serial.log", BOOT_TIMEOUT, output / f"{name}.commands.jsonl")
        try:
            uart.until()
            # Job 1 is the boot's V7 mount. With a fault armed, its success shows
            # that no write reached the armed event before the operation.
            uart.send(b"job-status 1\r")
            startup = uart.until()
            if "status=0" not in startup:
                raise AssertionError(f"{name}: V7 mount job did not succeed: {startup!r}")
            yield uart
            uart.send(b"exit\r")
            uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
            if vm.wait(timeout=10) != 33:
                raise RuntimeError(f"{name}: unclean terminal-v7 exit")
        finally:
            uart.close()


def _mem(uart):
    return decode_mem(uart.command("mem", "free_frames="))


def _write(uart, refs, version, epoch, key, seed, size, suffix=""):
    started = time.monotonic()
    text = uart.command(command(refs["workspace"], refs["resource"], version, epoch, key, seed, size) + suffix, "\n")
    result = decode_probe(text) if suffix.startswith(" probe") else decode(text)
    result["host_seconds"] = round(time.monotonic() - started, 3)
    return result


def _lookup_key(uart, refs, epoch, key):
    """Look a retry key up: the receipt, or the single refusal."""
    text = uart.command(f"operation-v7 {refs['workspace']} e_{hex16(epoch)} k_{hex16(key)}", "\n")
    if "\nerror:" in text.replace("\r\n", "\n"):
        return {"error": lookup_error(text)}
    return decode(text, LOOKUP_TIMING)


def _lookup_id(uart, operation):
    text = uart.command(f"operation-v7 {operation}", "\n")
    if "\nerror:" in text.replace("\r\n", "\n"):
        return {"error": lookup_error(text)}
    return decode(text, LOOKUP_TIMING)


def planned_sectors(snapshot, size):
    """Data sectors a fresh stage of `size` bytes writes, derived from an oracle-verified image.

    Mirrors the owner's planner (`plan_payload` in crates/fs/src/volume7/payload.rs)
    for the single-run case: the first of the largest free runs of the allocation
    map, which oracle7 has already proved equal to the sectors live files and
    non-aliasing snapshots own. With no other stage open, that is the plan.
    """
    claimed = set()
    for node in snapshot["nodes"].values():
        if node["kind"] == "file":
            for start, count in node["runs"]:
                claimed.update(range(start, start + count))
    for record in snapshot["records"]:
        if not record["aliases_live"]:
            for start, count in record["runs"]:
                claimed.update(range(start, start + count))
    best, cursor = None, 0
    while cursor < oracle7.DATA_SECTORS:
        while cursor < oracle7.DATA_SECTORS and cursor in claimed:
            cursor += 1
        start = cursor
        while cursor < oracle7.DATA_SECTORS and cursor not in claimed:
            cursor += 1
        if cursor > start and (best is None or cursor - start > best[1]):
            best = (start, cursor - start)
    sectors = -(-size // SECTOR)
    if best is None or best[1] < sectors:
        raise ValueError("the payload plan would need several runs; not modelled")
    return list(range(best[0], best[0] + sectors))


def payload_reached(image, sectors, content, cut):
    """Planned sectors before `cut` hold the pattern on the image; the cut sector does not.

    Returns the number of payload sectors found. For cuts after the payload,
    every planned sector must hold its part of the pattern.
    """
    written = min(cut, len(sectors))
    for index, sector in enumerate(sectors[:written + 1] if written < len(sectors) else sectors):
        first = (oracle7.PAYLOAD_SECTOR + sector) * SECTOR
        expected = content[index * SECTOR:(index + 1) * SECTOR].ljust(SECTOR, b"\0")
        present = bytes(image[first:first + SECTOR]) == expected
        if index < written and not present:
            raise AssertionError(f"payload sector {index} (data sector {sector}) did not reach the image")
        if index == written and present:
            raise AssertionError(f"payload sector {index} reached the image although its write failed")
    return written


def _scratch(snapshot, identity):
    return next(item for item in snapshot["files"] if item["id"] == identity)


def _record_keys(snapshot):
    return sorted((record["subject"], record["epoch"], record["key"], record["committed"])
                  for record in snapshot["records"])


def classify_write(base, observed, scratch_id, key, content):
    """Decide from two oracle views whether the interrupted write was published.

    The new generation must carry exactly one new shell record for `key` whose
    committed version is the live version and whose snapshot is `content`; the
    old generation must be the base view unchanged. Anything else is a failure.
    """
    live, previous = _scratch(observed, scratch_id), _scratch(base, scratch_id)
    ours = [record for record in observed["records"] if record["subject"] == SHELL_SUBJECT and record["key"] == key]
    digest = hashlib.sha256(content).hexdigest()
    if observed["sequence"] == base["sequence"] + 1:
        if len(ours) != 1 or len(observed["records"]) != len(base["records"]) + 1:
            raise AssertionError("the new generation does not add exactly one record for the key")
        record = ours[0]
        expected = {"state": "direct_committed", "object": scratch_id, "previous": previous["version"],
                    "committed": observed["sequence"], "length": len(content), "sha256": digest}
        for field, value in expected.items():
            if record[field] != value:
                raise AssertionError(f"new record {field}={record[field]!r}, expected {value!r}")
        if (live["version"], live["size"], live["sha256"]) != (record["committed"], len(content), digest):
            raise AssertionError("the live file is not the new record's version and content")
        return True
    if observed["sequence"] != base["sequence"]:
        raise AssertionError(f"unexpected generation {observed['sequence']} after base {base['sequence']}")
    if ours or _record_keys(observed) != _record_keys(base):
        raise AssertionError("the old generation gained or lost a record")
    if live != previous or observed["files"] != base["files"] or observed["free_sectors"] != base["free_sectors"]:
        raise AssertionError("the old generation's files or free space changed")
    return False


def classify_maintenance(base, observed):
    """Decide from two oracle views whether the interrupted maintenance was published."""
    if observed["epoch"] == base["epoch"] + 1:
        check_maintained(base, observed, {"previous": base["epoch"], "epoch": base["epoch"] + 1,
                                          "records": len(base["records"]), "sectors": reclaimable(base)})
        return True
    if (observed["epoch"], observed["sequence"]) != (base["epoch"], base["sequence"]):
        raise AssertionError("maintenance left an unexpected epoch or generation")
    if _record_keys(observed) != _record_keys(base) or observed["files"] != base["files"] \
            or observed["free_sectors"] != base["free_sectors"]:
        raise AssertionError("the old generation's records, files or free space changed")
    return False


def _same_receipt(found, expected, what):
    if "error" in found or found["lines"] != expected["lines"]:
        raise AssertionError(f"{what}: {found} differs from {expected['lines']}")


def _view(snapshot):
    return {"sequence": snapshot["sequence"], "generation": snapshot["generation"], "epoch": snapshot["epoch"],
            "records": len(snapshot["records"]), "free_sectors": snapshot["free_sectors"],
            "recovered": snapshot["recovered"]}


class Plan:
    """Shared inputs of every boot in one run."""

    def __init__(self, image, output, temporary, lineage, seeded):
        self.image, self.output, self.temporary = image, output, temporary
        self.lineage, self.seeded = lineage, seeded
        self.scratch = seeded["scratch"]
        self.refs = {"workspace": seeded["workspace"]["text"], "resource": self.scratch["resource"]}
        self.boots = 0
        self.memory = []

    def boot(self, data, name, fault_rules=None):
        self.boots += 1
        return _session(self.image, data, self.output, name, self.temporary, fault_rules)

    def mem(self, uart, boot, point):
        observed = _mem(uart)
        self.memory.append({"boot": boot, "point": point, **observed})
        return observed


def _fenced(uart, plan, name, refs, epoch, key):
    """While fenced: owner-local queries answer; the file service reports Uncertain."""
    uart.command("services", "files pid=")
    fenced = plan.mem(uart, name, "fenced")
    if fenced["pending_io"] != 0:
        raise AssertionError(f"{name}: I/O still pending after the failed operation")
    answer = _lookup_key(uart, refs, epoch, key)
    if answer.get("error") != "Uncertain":
        raise AssertionError(f"{name}: a lookup on the fenced volume was not Uncertain: {answer}")
    uart.command("restart files", "utility sessions revoked")
    plan.mem(uart, name, "after_restart")
    return {"services_answered": True, "fenced_lookup": "Uncertain"}


def _write_case(plan, base_bytes, base, name, cut):
    events = write_events(WRITE_SIZE)
    expected = adopts_new_generation(events, cut)
    content = pattern(WRITE_SEED, WRITE_SIZE)
    refs, scratch = plan.refs, plan.scratch
    version, epoch = scratch["version"], base["epoch"]
    data = plan.temporary / f"write-{name}.raw"
    data.write_bytes(base_bytes)
    sectors = planned_sectors(base, WRITE_SIZE)
    case = {"case": f"write_{name}", "operation": "write", "cut": cut, "event": events[cut],
            "events": len(events), "expected_new_generation": expected}
    fault = f"write-{name}-fault"
    with plan.boot(data, fault, rules(events, cut)) as uart:
        plan.mem(uart, fault, "idle")
        attempt = _write(uart, refs, version, epoch, WRITE_KEY, WRITE_SEED, WRITE_SIZE)
        case["status"] = attempt.get("error", "committed")
        if case["status"] != "Uncertain":
            raise AssertionError(f"{fault}: the interrupted write reported {attempt}")
        case.update(_fenced(uart, plan, fault, refs, epoch, WRITE_KEY))
        image = data.read_bytes()
        observed = oracle7.snapshot(image)
        adopted = classify_write(base, observed, scratch["id"], WRITE_KEY, content)
        # Before the reboot's retry can reuse the free run: the payload writes
        # before the cut are on the image, the failed one is not.
        case["payload_sectors_on_image"] = payload_reached(image, sectors, content, cut)
        if adopted and observed["nodes"][scratch["id"]]["runs"] != [(sectors[0], len(sectors))]:
            raise AssertionError(f"{fault}: the published file is not in the planned run")
        case["new_generation"] = adopted
        case["after_restart"] = _view(observed)
        if adopted != expected:
            raise AssertionError(f"{fault}: adopted {'new' if adopted else 'old'} generation at cut {cut}")
        found = _lookup_key(uart, refs, epoch, WRITE_KEY)
        if adopted:
            if "error" in found:
                raise AssertionError(f"{fault}: the published write has no receipt: {found}")
            check_receipt(found, plan.lineage, refs["workspace"], refs["resource"], version, epoch, WRITE_KEY, content)
            case["lookup"] = "receipt"
        elif found.get("error") != "OutcomeUnknown":
            raise AssertionError(f"{fault}: the unpublished write has an outcome: {found}")
        else:
            case["lookup"] = "OutcomeUnknown"
        head = read_range(uart, refs, 0, 1024)["result"]
        live = _scratch(observed, scratch["id"])
        if head["version"] != f"v_{hex16(live['version'])}" or head["size"] != live["size"]:
            raise AssertionError(f"{fault}: the guest reads another version than the image holds: {head}")
    before_reboot = environment.digest(data)
    reboot = f"write-{name}-reboot"
    with plan.boot(data, reboot) as uart:
        plan.mem(uart, reboot, "idle")
        again = _lookup_key(uart, refs, epoch, WRITE_KEY)
        if adopted:
            _same_receipt(again, found, f"{reboot}: lookup after reboot")
        elif again.get("error") != "OutcomeUnknown":
            raise AssertionError(f"{reboot}: the unpublished write has an outcome after reboot: {again}")
        retry = _write(uart, refs, version, epoch, WRITE_KEY, WRITE_SEED, WRITE_SIZE)
        if "error" in retry:
            raise AssertionError(f"{reboot}: the exact retry failed: {retry}")
        check_receipt(retry, plan.lineage, refs["workspace"], refs["resource"], version, epoch, WRITE_KEY, content)
        if adopted:
            _same_receipt(retry, found, f"{reboot}: replay")
        elif retry["version"] != base["sequence"] + 1:
            raise AssertionError(f"{reboot}: the retry committed version {retry['version']}")
        case["retry"] = "replayed" if adopted else "committed"
        plan.mem(uart, reboot, "after_retry")
    after_reboot = environment.digest(data)
    if adopted and after_reboot != before_reboot:
        raise AssertionError(f"{reboot}: the lookup or replay wrote to the volume")
    case["reboot_image_unchanged"] = after_reboot == before_reboot
    final = oracle7.snapshot(data.read_bytes())
    if not classify_write(base, final, scratch["id"], WRITE_KEY, content):
        raise AssertionError(f"{reboot}: the retry left the write unpublished")
    match_records(final, [retry], plan.seeded["workspace"]["id"], scratch["id"])
    case["final"] = _view(final)
    case["verified"] = True
    return case


def _maintenance_case(plan, base_bytes, base, last, name, cut):
    events = publication_events()
    expected = adopts_new_generation(events, cut)
    refs = plan.refs
    data = plan.temporary / f"maintain-{name}.raw"
    data.write_bytes(base_bytes)
    case = {"case": f"maintain_{name}", "operation": "maintenance", "cut": cut, "event": events[cut],
            "events": len(events), "expected_new_generation": expected}
    fault = f"maintain-{name}-fault"

    def old_outcomes(uart, adopted, boot):
        by_key = _lookup_key(uart, refs, base["epoch"], last["key"])
        by_id = _lookup_id(uart, last["id"])
        if adopted:
            if (by_key.get("error"), by_id.get("error")) != ("ExpiredEpoch", "OutcomeUnknown"):
                raise AssertionError(f"{boot}: reclaimed outcomes answered {by_key} / {by_id}")
            return {"key": "ExpiredEpoch", "id": "OutcomeUnknown"}
        _same_receipt(by_key, last, f"{boot}: retained lookup by key")
        _same_receipt(by_id, last, f"{boot}: retained lookup by id")
        return {"key": "receipt", "id": "receipt"}

    with plan.boot(data, fault, rules(events, cut)) as uart:
        plan.mem(uart, fault, "idle")
        answer = decode_maintain(uart.command("maintain-v7", "\n"))
        case["status"] = answer.get("error", "maintained")
        if case["status"] != "Uncertain":
            raise AssertionError(f"{fault}: the interrupted maintenance reported {answer}")
        case.update(_fenced(uart, plan, fault, refs, base["epoch"], last["key"]))
        observed = oracle7.snapshot(data.read_bytes())
        adopted = classify_maintenance(base, observed)
        case["new_generation"] = adopted
        case["after_restart"] = _view(observed)
        if adopted != expected:
            raise AssertionError(f"{fault}: adopted {'new' if adopted else 'old'} generation at cut {cut}")
        case["old_outcomes"] = old_outcomes(uart, adopted, fault)
    before_reboot = environment.digest(data)
    reboot = f"maintain-{name}-reboot"
    with plan.boot(data, reboot) as uart:
        plan.mem(uart, reboot, "idle")
        if old_outcomes(uart, adopted, reboot) != case["old_outcomes"]:
            raise AssertionError(f"{reboot}: outcomes changed across the reboot")
        if environment.digest(data) != before_reboot:
            raise AssertionError(f"{reboot}: mount or lookups wrote to the volume")
        again = decode_maintain(uart.command("maintain-v7", "\n"))
        if "error" in again:
            raise AssertionError(f"{reboot}: the repeated maintenance failed: {again}")
        case["repeat"] = {key: again[key] for key in ("previous", "epoch", "records", "sectors", "ticks")}
        plan.mem(uart, reboot, "after_repeat")
    final = oracle7.snapshot(data.read_bytes())
    check_maintained(observed, final, again)
    case["final"] = _view(final)
    case["verified"] = True
    return case


def _budget(plan, data, base):
    """Memory and control latency around large writes and one owner revocation."""
    refs, scratch, epoch = plan.refs, plan.scratch, base["epoch"]
    version = scratch["version"]
    budget = {"writes": []}
    name = "budget"
    with plan.boot(data, name) as uart:
        budget["idle"] = plan.mem(uart, name, "idle")
        receipts = []
        for index, seed in enumerate(BUDGET_SEEDS):
            written = _write(uart, refs, version, epoch, BUDGET_KEY + index, seed, LARGE_SIZE,
                             f" probe {PROBE_EVERY}")
            if "error" in written:
                raise AssertionError(f"budget write {index} failed: {written}")
            check_receipt(written, plan.lineage, refs["workspace"], refs["resource"], version, epoch,
                          BUDGET_KEY + index, pattern(seed, LARGE_SIZE))
            probe = written["probe"]
            if probe["probes"] != expected_probes(LARGE_SIZE, PROBE_EVERY) or probe["every"] != PROBE_EVERY:
                raise AssertionError(f"the probe took {probe['probes']} samples")
            after = plan.mem(uart, name, f"after_write_{index}")
            budget["writes"].append({"size": LARGE_SIZE, "ticks": written["ticks"],
                                     "host_seconds": written["host_seconds"], "probe": probe,
                                     "after_commit": after})
            receipts.append({**written, "seed": seed})
            version = written["version"]
        small = _write(uart, refs, version, epoch, BUDGET_KEY + len(BUDGET_SEEDS), SMALL_SEED, SMALL_SIZE)
        if "error" in small:
            raise AssertionError(f"budget small write failed: {small}")
        receipts.append({**small, "seed": SMALL_SEED})
        version = small["version"]
        text = uart.command(command(refs["workspace"], refs["resource"], version, epoch, CUT_KEY, 1, LARGE_SIZE)
                            + f" cut {CUT_CHUNKS}", "\n")
        cut = decode_cut(text)
        if "error" in cut or (cut["old"], cut["new"]) != ("Closed", "NoTransfer"):
            raise AssertionError(f"the owner revocation was not observed: {cut}")
        budget["revocation"] = cut
        budget["after_revocation"] = plan.mem(uart, name, "after_revocation")
    idle = budget["idle"]
    for point in [write["after_commit"] for write in budget["writes"]] + [budget["after_revocation"]]:
        if (point["free_frames"], point["heap_pages"]) != (idle["free_frames"], idle["heap_pages"]):
            raise AssertionError(f"system memory did not return to the idle baseline: {point} vs {idle}")
    for write in budget["writes"]:
        probe = write["probe"]
        if (probe["free_min"], probe["heap_max"]) != (idle["free_frames"], idle["heap_pages"]) \
                or probe["free_max"] != idle["free_frames"] or probe["heap_min"] != idle["heap_pages"]:
            raise AssertionError(f"memory changed while a transfer was open: {probe} vs {idle}")
    snapshot = oracle7.snapshot(data.read_bytes())
    match_records(snapshot, receipts, plan.seeded["workspace"]["id"], scratch["id"])
    return budget, receipts, snapshot


def _mount_only(plan, data):
    before = environment.digest(data)
    name = "mount-only"
    with plan.boot(data, name) as uart:
        plan.mem(uart, name, "idle")
    after = environment.digest(data)
    if after != before:
        raise AssertionError("a boot that only mounted the V7 volume changed the image")
    return {"sha256": after, "unchanged": True}


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-faults")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    for path, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(path) != metadata["native_applications"]["file-server"][suffix]:
            raise RuntimeError(f"{path.name} differs from the artifact recorded by the boot build")
    (output / "result.json").unlink(missing_ok=True)
    for stale in list(output.glob("*.serial.log")) + list(output.glob("*.qemu.log")) + list(output.glob("*.commands.jsonl")):
        stale.unlink()
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-faults-") as temporary:
        temporary = Path(temporary)
        seed = temporary / "seed.raw"
        lineage = uuid.uuid4().hex
        seeded = volume_json(volume_tool, "seed7", seed, lineage, elf, manifest, "--scratch")
        plan = Plan(image, output, temporary, lineage, seeded)
        write_base = seed.read_bytes()
        write_view = oracle7.snapshot(write_base)

        mount_data = temporary / "mount-only.raw"
        mount_data.write_bytes(write_base)
        mount_only = _mount_only(plan, mount_data)

        budget_data = temporary / "budget.raw"
        budget_data.write_bytes(write_base)
        budget, receipts, maintenance_view = _budget(plan, budget_data, write_view)
        maintenance_base = budget_data.read_bytes()

        cases = []
        for name, cut in write_cuts(WRITE_SIZE).items():
            cases.append(_write_case(plan, write_base, write_view, name, cut))
        for name, cut in maintenance_cuts().items():
            cases.append(_maintenance_case(plan, maintenance_base, maintenance_view, receipts[-1], name, cut))

    idle = [item for item in plan.memory if item["point"] == "idle"]
    baseline = budget["idle"]
    # Every observation, fenced ones included, must equal the first boot's idle baseline.
    returned = plan.memory
    drift = [item for item in returned
             if (item["free_frames"], item["heap_pages"]) != (baseline["free_frames"], baseline["heap_pages"])]
    if drift:
        raise AssertionError(f"system memory differs from the idle baseline: {drift[:3]}")
    static = static_pages(elf.read_bytes())
    probes = [write["probe"] for write in budget["writes"]]
    evidence = {
        "verified": True,
        "mode": "terminal-v7",
        "boots": plan.boots,
        "lineage": lineage,
        "write": {"size": WRITE_SIZE, "events": len(write_events(WRITE_SIZE)), "key": WRITE_KEY},
        "maintenance": {"events": len(publication_events()), "records": len(maintenance_view["records"]),
                        "reclaimable_sectors": reclaimable(maintenance_view)},
        "mount_only": mount_only,
        "cases": cases,
        "memory": {
            "baseline": {key: baseline[key] for key in ("free_frames", "heap_pages", "processes", "channels")},
            "observations": plan.memory,
            "returned_to_baseline": len(returned),
            "idle_boots": len(idle),
            "file_server_static": static,
        },
        "latency": {
            "info_round_trip_ticks": [{"max": probe["max"], "p50": probe["p50"], "total": probe["total"],
                                       "probes": probe["probes"]} for probe in probes],
            "large_write_ticks": [write["ticks"] for write in budget["writes"]],
            "revocation_ticks": budget["revocation"]["ticks"],
        },
        "budget": budget,
    }
    result = {
        "outcome": "success",
        "returncode": 33,
        "timed_out": False,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "build_id": metadata["build_id"],
        "image_sha256": metadata["image_sha256"],
        "terminal_v7_faults": evidence,
    }
    (output / "result.json").write_text(json.dumps(result, indent=1) + "\n")
    print(f"V7 faults: {len(cases)} interrupted publications ({plan.boots} boots, "
          f"{result['elapsed_seconds']} s); every operation Uncertain, restart and reboot agree with oracle7; "
          "only final-flush cuts adopt the new generation; mount-only boot left the image unchanged.", flush=True)
    for case in cases:
        print(f"V7 cut: {case['case']} index={case['cut']} event={case['event']} status={case['status']} "
              f"generation={'new' if case['new_generation'] else 'old'}", flush=True)
    print(f"V7 memory: free_frames={baseline['free_frames']} heap_pages={baseline['heap_pages']} at every idle, "
          f"fenced, restart and post-operation point ({len(returned)} points); file-server static bound "
          f"{static['pages']} pages ({static['load_pages']} PT_LOAD + {STACK_PAGES} stack) of {IMAGE_PAGE_BUDGET}",
          flush=True)
    for probe in probes:
        print(f"V7 INFO during 512 KiB write: probes={probe['probes']} max={probe['max']} p50={probe['p50']} "
              f"total={probe['total']} ticks", flush=True)
    print(f"V7 revocation: guest_ticks={budget['revocation']['ticks']}", flush=True)
    return result
