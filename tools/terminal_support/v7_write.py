# SPDX-License-Identifier: Apache-2.0
"""Profile-2 tracked writes to a fresh V7 volume in two disposable UEFI boots.

Boot 1 streams deterministic patterns of increasing size into the seeded
`scratch.bin` until the eight-record budget is exhausted, checks a stale
version and the `Full` refusal. Before the last write, with exactly one record
slot left, a 512 KiB write is cut after enough acknowledged chunks for more
than twenty sectors to have been staged (inferred from the acknowledged bytes)
and the owner revokes the shell's binding: the old endpoint is
`Closed`, the same transfer on the new binding is `NoTransfer`, a read still
shows the previous version, and the last write then takes the last slot, which
it could not if the revoked stage still held its reservation. Boot 2 replays
one earlier write exactly and expects a byte-identical receipt, checks that a
mismatched retry is an idempotency conflict, and looks up a small and a large
write cold by operation ID and by retry key: each lookup must print exactly the
lines printed at commit, and the revoked key has no outcome. The volume digest
is unchanged across boot 2. After each boot the independent `oracle7` reader
must find records whose fields and SHA-256 match the receipts the guest
printed, and none for the revoked key.
"""
import hashlib
import json
from pathlib import Path
import re
import tempfile
import time
import uuid

import environment
from . import oracle7
from .connection import Connection
from .machine import machine
from .read_cases import read as read_range
from .v7_read import volume_json


ROOT = environment.ROOT
BOOT_TIMEOUT = 300
# Shell subject of the V7 policy in apps/supervisor/src/work/mount.rs.
SHELL_SUBJECT = 2
RETAINED = 8
# Records `rustic-volume seed7 --scratch` provisions (ELF and manifest). The
# revocation case relies on exactly one free slot being left before the last
# write, so the seed must hold exactly this many.
SEEDED_RECORDS = 2
# Sizes for boot 1; the seed already holds two records, so six fit the budget.
SIZES = (513, 8 * 1024, 64 * 1024, 1, 256 * 1024, 512 * 1024)
FIRST_KEY = 0x100
REPLAY_INDEX = 1
# Writes looked up cold in boot 2: the smallest multi-sector and the largest.
LOOKUP_INDEXES = (0, 5)
# The revoked write: before the last write, cut after 400 chunks (16,000
# bytes, 31 staged sectors) of the same 512 KiB pattern under its own key.
CUT_BEFORE = 5
CUT_KEY = 0x180
CUT_CHUNKS = 400
CHUNK_BYTES = 40

OPERATION = re.compile(
    r"^operation-v1 id=(op_[0-9a-f]{32}_[0-9a-f]{16}) service_instance=(si_[0-9a-f]{32}_[0-9a-f]{16}) "
    r"state=succeeded effect=committed cancel_requested=false$"
)
RECEIPT = re.compile(
    r"^receipt workspace=(ws_[0-9a-f]{32}_[0-9a-f]{8}) resource=(rs_[0-9a-f]{32}_[0-9a-f]{8}_[0-9a-f]{8}) "
    r"previous_version=v_([0-9a-f]{16}) version=v_([0-9a-f]{16}) size=(0|[1-9][0-9]{0,6}) "
    r"epoch=e_([0-9a-f]{16}) key=k_([0-9a-f]{16}) sha256=([0-9a-f]{64})$"
)
TIMING = re.compile(r"^write-v7 size=(0|[1-9][0-9]{0,6}) ticks=(0|[1-9][0-9]{0,15})$")
LOOKUP_TIMING = re.compile(r"^lookup-v7 size=(0|[1-9][0-9]{0,6}) ticks=(0|[1-9][0-9]{0,15})$")
CUT = re.compile(r"^cut-v7 chunks=([1-9][0-9]{0,6}) bytes=([1-9][0-9]{0,6}) job=([1-9][0-9]{0,15}) "
                 r"old=([A-Za-z]+) new=([A-Za-z]+)$")
ERROR = re.compile(r"^error: ([A-Za-z]+)$")


def pattern(seed, size):
    """The shell's `replace-pattern-v7` content: s*31 + 7*i + i//509 mod 256."""
    base = (seed * 31) & 0xFF
    return bytes((base + 7 * index + index // 509) & 0xFF for index in range(size))


def hex16(value):
    return f"{value:016x}"


def command(workspace, resource, version, epoch, key, seed, size):
    return (f"replace-pattern-v7 {workspace} {resource} v_{hex16(version)} e_{hex16(epoch)} "
            f"k_{hex16(key)} {seed} {size}")


def decode(text, timing=TIMING):
    """Decode one complete `replace-pattern-v7` answer: a receipt or one error.

    With `timing=LOOKUP_TIMING` it decodes an `operation-v7` lookup instead.
    """
    # The echoed command and the trailing prompt carry no result.
    prefix = timing.pattern[1:].split(" ", 1)[0]
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    lines = [line for line in lines if line.startswith(("operation-v1", "receipt", prefix, "error"))]
    errors = [ERROR.match(line) for line in lines if line.startswith("error")]
    if errors:
        if len(errors) != 1 or not errors[0] or any(line.startswith(("operation-v1", "receipt")) for line in lines):
            raise ValueError(f"ambiguous write failure: {text!r}")
        return {"error": errors[0][1]}
    found = [(OPERATION.match(line), RECEIPT.match(line), timing.match(line)) for line in lines]
    operations = [match[0] for match in found if match[0]]
    receipts = [match[1] for match in found if match[1]]
    timings = [match[2] for match in found if match[2]]
    if len(operations) != 1 or len(receipts) != 1 or len(timings) != 1:
        raise ValueError(f"incomplete or ambiguous write receipt: {text!r}")
    receipt = receipts[0]
    if int(timings[0][1]) != int(receipt[5]):
        raise ValueError("timing line names another size")
    operation_line = next(line for line in lines if line.startswith("operation-v1"))
    receipt_line = next(line for line in lines if line.startswith("receipt"))
    return {
        "id": operations[0][1],
        "service_instance": operations[0][2],
        "workspace": receipt[1],
        "resource": receipt[2],
        "previous": int(receipt[3], 16),
        "version": int(receipt[4], 16),
        "size": int(receipt[5]),
        "epoch": int(receipt[6], 16),
        "key": int(receipt[7], 16),
        "sha256": receipt[8],
        "ticks": int(timings[0][2]),
        "lines": [operation_line, receipt_line],
    }


def decode_cut(text):
    """Decode one `replace-pattern-v7 ... cut N` answer: the observations or one error."""
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    lines = [line for line in lines if line.startswith(("operation-v1", "receipt", "write-v7", "cut-v7", "error"))]
    if len(lines) != 1:
        raise ValueError(f"incomplete or ambiguous cut answer: {text!r}")
    error = ERROR.match(lines[0])
    if error:
        return {"error": error[1]}
    match = CUT.match(lines[0])
    if not match:
        raise ValueError(f"malformed cut answer: {text!r}")
    return {"chunks": int(match[1]), "bytes": int(match[2]), "job": int(match[3]),
            "old": match[4], "new": match[5]}


def lookup_error(text):
    """The single refusal of an `operation-v7` lookup."""
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    lines = [line for line in lines if line.startswith(("operation-v1", "receipt", "lookup-v7", "error"))]
    if len(lines) != 1 or not ERROR.match(lines[0]):
        raise ValueError(f"lookup was expected to be refused: {text!r}")
    return ERROR.match(lines[0])[1]


def _sequence(text, prefix):
    """The trailing 16-hex-digit sequence of an `op_`/`si_` identifier."""
    if not text.startswith(prefix):
        raise ValueError(f"{text!r} is not a {prefix} identifier")
    return int(text.rsplit("_", 1)[1], 16)


def check_receipt(receipt, lineage, workspace, resource, previous, epoch, key, content):
    """Verify one printed receipt against the request and the host content."""
    expected = {
        "workspace": workspace, "resource": resource, "previous": previous,
        "epoch": epoch, "key": key, "size": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
    }
    for field, value in expected.items():
        if receipt[field] != value:
            raise AssertionError(f"receipt {field}={receipt[field]!r}, expected {value!r}")
    if receipt["id"] != f"op_{lineage}_{hex16(receipt['version'])}":
        raise AssertionError("operation id does not name the committed version")
    if not receipt["service_instance"].startswith(f"si_{lineage}_"):
        raise AssertionError("service instance names another lineage")
    if receipt["version"] <= previous:
        raise AssertionError("committed version does not advance")


def match_records(snapshot, receipts, workspace_id, object_id):
    """Every guest receipt must equal one oracle-verified retained record."""
    records = [record for record in snapshot["records"] if record["subject"] == SHELL_SUBJECT]
    by_key = {record["key"]: record for record in records}
    if len(by_key) != len(records):
        raise AssertionError("two shell records share a retry key")
    matched = []
    for receipt in receipts:
        record = by_key.get(receipt["key"])
        if record is None:
            raise AssertionError(f"no retained record for key {receipt['key']:#x}")
        expected = {
            "state": "direct_committed", "workspace": workspace_id, "object": object_id,
            "epoch": receipt["epoch"], "previous": receipt["previous"],
            "committed": receipt["version"], "terminal": receipt["version"],
            "length": receipt["size"], "sha256": receipt["sha256"],
            "instance": _sequence(receipt["service_instance"], "si_"),
        }
        for field, value in expected.items():
            if record[field] != value:
                raise AssertionError(f"record {field}={record[field]!r}, receipt says {value!r}")
        matched.append({"slot": record["slot"], "key": record["key"], "committed": record["committed"]})
    return matched


def check_absent(snapshot, key):
    """The revoked key must have left no retained record of the shell."""
    if any(record["subject"] == SHELL_SUBJECT and record["key"] == key for record in snapshot["records"]):
        raise AssertionError(f"a retained record exists for the revoked key {key:#x}")


def _write(uart, refs, version, epoch, key, seed, size):
    started = time.monotonic()
    text = uart.command(command(refs["workspace"], refs["resource"], version, epoch, key, seed, size),
                        "\n")
    result = decode(text)
    result["host_seconds"] = round(time.monotonic() - started, 3)
    return result


def _boot(image, data, output, phase, temporary, body):
    sock = temporary / f"uart-{phase}.sock"
    transcript = output / f"serial-{phase}.log"
    log = output / f"qemu-{phase}.log"
    with machine(image, data, f"unix:{sock},server=on,wait=off", log) as vm:
        uart = Connection(sock, vm, transcript, BOOT_TIMEOUT, output / f"commands-{phase}.jsonl")
        try:
            uart.until()
            if phase == 1:
                uart.send(b"job-status 1\r")
                startup = uart.until()
                if "status=0" not in startup:
                    raise AssertionError(f"initial V7 mount job did not succeed: {startup!r}")
            result = body(uart)
            uart.send(b"exit\r")
            uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
            if vm.wait(timeout=10) != 33:
                raise RuntimeError("unclean terminal-v7 exit")
            return result
        finally:
            uart.close()


def _first_boot(uart, refs, lineage, epoch, version, seeded_records):
    writes = []
    budget = RETAINED - seeded_records
    if budget != len(SIZES):
        raise AssertionError(f"seed left {budget} records, the plan writes {len(SIZES)}")
    stale = _write(uart, refs, version + 1, epoch, FIRST_KEY - 1, 1, 64)
    if stale != {"error": "Version", "host_seconds": stale["host_seconds"]}:
        raise AssertionError(f"future version was not refused: {stale}")
    cut = None
    for index, size in enumerate(SIZES):
        key, seed = FIRST_KEY + index, index + 1
        if index == CUT_BEFORE:
            cut = _cut(uart, refs, version, epoch, seed, size, writes[-1])
        receipt = _write(uart, refs, version, epoch, key, seed, size)
        if "error" in receipt:
            raise AssertionError(f"write {index} of {size} bytes failed: {receipt['error']}")
        check_receipt(receipt, lineage, refs["workspace"], refs["resource"], version, epoch, key,
                      pattern(seed, size))
        writes.append({**receipt, "seed": seed})
        version = receipt["version"]
    stale = _write(uart, refs, writes[0]["previous"], epoch, FIRST_KEY + len(SIZES), 9, 64)
    if stale.get("error") != "Version":
        raise AssertionError(f"stale version was not refused: {stale}")
    full = _write(uart, refs, version, epoch, FIRST_KEY + len(SIZES), 9, 64)
    if full.get("error") != "Full":
        raise AssertionError(f"write beyond the retained budget was not Full: {full}")
    # The guest read path observes the last committed bytes.
    last = writes[-1]
    head = read_range(uart, refs, 0, 1024)["result"]
    if head["version"] != f"v_{hex16(last['version'])}" or head["size"] != last["size"]:
        raise AssertionError("guest read does not observe the last committed write")
    return {"writes": writes, "stale_version": "Version", "beyond_budget": "Full", "cut": cut}


def _cut(uart, refs, version, epoch, seed, size, previous):
    """Cut a write mid-transfer, revoke the binding and observe both ends."""
    started = time.monotonic()
    text = uart.command(command(refs["workspace"], refs["resource"], version, epoch, CUT_KEY, seed, size)
                        + f" cut {CUT_CHUNKS}", "\n")
    cut = decode_cut(text)
    cut["host_seconds"] = round(time.monotonic() - started, 3)
    expected = {"chunks": CUT_CHUNKS, "bytes": CUT_CHUNKS * CHUNK_BYTES, "old": "Closed", "new": "NoTransfer"}
    if "error" in cut or any(cut[field] != value for field, value in expected.items()):
        raise AssertionError(f"revocation mid-transfer was not observed as expected: {cut}")
    # Inferred from the acknowledged bytes: every full 512-byte sector the
    # service acknowledged was handed to the stage.
    cut["staged_sectors_inferred"] = cut["bytes"] // 512
    if cut["staged_sectors_inferred"] < 20:
        raise AssertionError("the cut acknowledged fewer than twenty sectors")
    # The new binding reads the file at the version before the cut.
    head = read_range(uart, refs, 0, 1024)["result"]
    if head["version"] != f"v_{hex16(previous['version'])}" or head["size"] != previous["size"]:
        raise AssertionError(f"guest read after revocation does not show the previous version: {head}")
    cut["read_version"] = previous["version"]
    cut["key"] = CUT_KEY
    return cut


def _lookups(uart, refs, epoch, writes):
    """Cold lookups print exactly the commit-time receipt lines."""
    found = []
    for index in LOOKUP_INDEXES:
        prior = writes[index]
        for name, query in (("id", prior["id"]),
                            ("retry", f"{refs['workspace']} e_{hex16(epoch)} k_{hex16(prior['key'])}")):
            started = time.monotonic()
            result = decode(uart.command(f"operation-v7 {query}", "\n"), LOOKUP_TIMING)
            if "error" in result:
                raise AssertionError(f"lookup by {name} of write {index} failed: {result['error']}")
            if result["lines"] != prior["lines"]:
                raise AssertionError(f"lookup by {name} differs: {result['lines']} != {prior['lines']}")
            found.append({"index": index, "by": name, "size": result["size"], "ticks": result["ticks"],
                          "host_seconds": round(time.monotonic() - started, 3), "identical_receipt": True})
    revoked = lookup_error(uart.command(f"operation-v7 {refs['workspace']} e_{hex16(epoch)} k_{hex16(CUT_KEY)}",
                                        "error:"))
    if revoked != "OutcomeUnknown":
        raise AssertionError(f"the revoked key has an outcome: {revoked}")
    return {"lookups": found, "revoked_key": revoked}


def _second_boot(uart, refs, epoch, prior, writes):
    replay = _write(uart, refs, prior["previous"], epoch, prior["key"], prior["seed"], prior["size"])
    if "error" in replay:
        raise AssertionError(f"exact retry failed: {replay['error']}")
    if replay["lines"] != prior["lines"]:
        raise AssertionError(f"replayed receipt differs: {replay['lines']} != {prior['lines']}")
    conflicts = {}
    for name, seed, size in (("different_bytes", prior["seed"] + 100, prior["size"]),
                             ("different_size", prior["seed"], prior["size"] + 1)):
        refused = _write(uart, refs, prior["previous"], epoch, prior["key"], seed, size)
        if refused.get("error") != "IdempotencyConflict":
            raise AssertionError(f"{name} retry was not an idempotency conflict: {refused}")
        conflicts[name] = "IdempotencyConflict"
    return {"replay": {"key": prior["key"], "size": prior["size"], "ticks": replay["ticks"],
                       "host_seconds": replay["host_seconds"], "identical_receipt": True},
            "conflicts": conflicts, **_lookups(uart, refs, epoch, writes)}


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-write")
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
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-write-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-volume.raw"
            lineage = uuid.uuid4().hex
            seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
            scratch = seeded["scratch"]
            refs = {"workspace": seeded["workspace"]["text"], "resource": scratch["resource"]}
            initial = oracle7.snapshot(data.read_bytes())
            epoch = initial["epoch"]
            seeded_records = len(initial["records"])
            if seeded_records != SEEDED_RECORDS:
                raise AssertionError(f"seed7 provisioned {seeded_records} records, the plan needs "
                                     f"{SEEDED_RECORDS} so that exactly one slot is left before the cut")

            first = _boot(image, data, output, 1, temporary,
                          lambda uart: _first_boot(uart, refs, lineage, epoch, scratch["version"], seeded_records))
            after_first = oracle7.snapshot(data.read_bytes())
            matched = match_records(after_first, first["writes"], seeded["workspace"]["id"], scratch["id"])
            check_absent(after_first, CUT_KEY)
            # Nothing was published between the write before the cut and the
            # write after it: the target stayed at the previous version.
            before_cut, after_cut = first["writes"][CUT_BEFORE - 1], first["writes"][CUT_BEFORE]
            if (after_cut["previous"], after_cut["version"]) != (before_cut["version"], before_cut["version"] + 1):
                raise AssertionError("the revoked write published a version")
            if len(after_first["records"]) != RETAINED:
                raise AssertionError("the retained budget is not exactly full after boot 1")
            last = first["writes"][-1]
            live = next(item for item in after_first["files"] if item["id"] == scratch["id"])
            if (live["version"], live["size"], live["sha256"]) != (last["version"], last["size"], last["sha256"]):
                raise AssertionError("the live scratch file is not the last committed pattern")
            for source in (elf, manifest):
                if after_first["contents"].get(f"/workspaces/application/{source.name}") != source.read_bytes():
                    raise AssertionError(f"boot 1 changed {source.name}")
            report = volume_json(volume_tool, "report7", data)
            if report["sequence"] != after_first["sequence"] or len(report["records"]) != RETAINED:
                raise AssertionError("report7 and oracle7 disagree after boot 1")

            before_second = environment.digest(data)
            prior = first["writes"][REPLAY_INDEX]
            second = _boot(image, data, output, 2, temporary,
                           lambda uart: _second_boot(uart, refs, epoch, prior, first["writes"]))
            after_second = environment.digest(data)
            if before_second != after_second:
                raise AssertionError("boot 2 replay or refusals changed the V7 volume")
            final = oracle7.snapshot(data.read_bytes())
            match_records(final, first["writes"], seeded["workspace"]["id"], scratch["id"])
            check_absent(final, CUT_KEY)

        timings = [{"size": item["size"], "guest_ticks": item["ticks"],
                    "guest_seconds": item["ticks"] / 100, "host_seconds": item["host_seconds"]}
                   for item in first["writes"]]
        evidence = {
            "verified": True,
            "mode": "terminal-v7",
            "boots": 2,
            "lineage": lineage,
            "subject": SHELL_SUBJECT,
            "scratch": scratch,
            "writes": [{key: item[key] for key in ("id", "service_instance", "previous", "version", "size",
                                                  "epoch", "key", "seed", "sha256", "lines")}
                       for item in first["writes"]],
            "timings": timings,
            "refusals": {"stale_version": first["stale_version"], "beyond_budget": first["beyond_budget"],
                         **second["conflicts"]},
            "replay": second["replay"],
            "revocation": first["cut"],
            "lookups": second["lookups"],
            "revoked_key_lookup": second["revoked_key"],
            "records": matched,
            "volume": {"sha256_before_boot_2": before_second, "sha256_after_boot_2": after_second,
                       "sequence": final["sequence"], "free_sectors": final["free_sectors"]},
        }
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "terminal_v7_write": evidence,
        }
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print("V7 write acceptance: six streamed tracked writes, stale Version, Full after the budget; "
              "a write cut after 31 sectors acknowledged (inferred from acked bytes) and revoked (old endpoint Closed, new binding NoTransfer, "
              "previous version readable, last slot still usable); exact replay after reboot with an "
              "identical receipt; cold lookups by ID and retry key identical to the commit receipts; "
              "unchanged volume across boot 2; mismatched retries IdempotencyConflict; oracle7 records "
              "match every receipt and none exists for the revoked key.", flush=True)
        for item in timings:
            print(f"V7 write: size={item['size']} guest_ticks={item['guest_ticks']} "
                  f"host_seconds={item['host_seconds']}", flush=True)
        for item in second["lookups"]:
            print(f"V7 lookup: by={item['by']} size={item['size']} guest_ticks={item['ticks']} "
                  f"host_seconds={item['host_seconds']}", flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(
            b"\n".join((output / f"serial-{p}.log").read_bytes() for p in phases if (output / f"serial-{p}.log").exists()))
        (output / "qemu.log").write_bytes(
            b"\n".join((output / f"qemu-{p}.log").read_bytes() for p in phases if (output / f"qemu-{p}.log").exists()))
