# SPDX-License-Identifier: Apache-2.0
"""Profile-2 staged admissions on a fresh V7 volume in two disposable UEFI boots.

Boot 1 admits a 64 KiB deterministic pattern into the seeded `scratch.bin`
with `admit-pattern-v7`: the file does not change, and the independent
`oracle7` reader finds one `admitted` record of the shell's subject whose
staged snapshot holds exactly the pattern. An exact retry prints the same
status and leaves the image byte-identical; status by retry identity, by
admission ID and the cause-aware observation all report it admitted. The
owner's `maintain-v7` is `Busy` while it is unresolved, with the image
unchanged.

Boot 2 finds the admission still admitted, executes it (the shell's own
binding is the executor), and looks the completion receipt up by operation ID
and by retry key: both print identical lines whose SHA-256 is the pattern's.
Execute and cancel replays print the same committed status without writing. A
second admission is then overtaken by a concurrent tracked write under another
key: execution is refused with `Version`, the record stays admitted and the
image unchanged; an explicit cancel makes it `cancelled`, the observation shows
the `requested` cause, and replays write nothing. With every admission
resolved the owner's maintenance succeeds and drops all records.

`oracle7` checks the image while the guest is idle after every phase (the
shell prints only after the service's publication and flushes) and again after
each clean shutdown.
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
from .v7_read import volume_json
from .v7_retention import check_maintained, decode_maintain, view
from .v7_write import (ERROR, LOOKUP_TIMING, SHELL_SUBJECT, boot_terminal, check_receipt, command, decode, hex16,
                       pattern)


ROOT = environment.ROOT
SEEDED_RECORDS = 2
LARGE = 64 * 1024
SMALL = 4096
TRACKED = 700
LARGE_KEY, SMALL_KEY, TRACKED_KEY = 0x400, 0x401, 0x402
LARGE_SEED, SMALL_SEED, TRACKED_SEED = 1, 2, 3

STATUS = re.compile(r"^admission-v1 id=(ad_([0-9a-f]{32})_([0-9a-f]{16})) "
                    r"service_instance=(si_([0-9a-f]{32})_([0-9a-f]{16})) "
                    r"state=(admitted|cancelled|committed) terminal=(0|[1-9][0-9]{0,19})$")
COMPLETION = re.compile(r"^completion=(op_([0-9a-f]{32})_([0-9a-f]{16}))$")
ADMIT_TIMING = re.compile(r"^admit-v7 size=(0|[1-9][0-9]{0,6}) ticks=(0|[1-9][0-9]{0,15})$")
OBSERVATION = re.compile(r"^admission-observation-v2 profile=2 id=(ad_[0-9a-f]{32}_[0-9a-f]{16}) "
                         r"service_instance=(si_[0-9a-f]{32}_[0-9a-f]{16}) kind=retained "
                         r"state=(admitted|cancelled|committed) terminal=(0|[1-9][0-9]{0,19}) "
                         r"prevention=(none|unknown|requested|version_conflict|authority_lost)$")
ORACLE_STATES = {"admitted": "admitted", "committed": "admitted_committed", "cancelled": "cancelled"}


def _lines(text, prefixes):
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    return [line for line in lines if line.startswith(prefixes)]


def decode_status(text, timing=False):
    """Decode one admission status answer (`admission-v1` plus its completion
    and, for `admit-pattern-v7`, its timing) or one error."""
    lines = _lines(text, ("admission-v1", "completion=", "admit-v7", "error"))
    errors = [line for line in lines if line.startswith("error")]
    if errors:
        if len(lines) != 1 or not ERROR.match(errors[0]):
            raise ValueError(f"ambiguous admission failure: {text!r}")
        return {"error": ERROR.match(errors[0])[1]}
    statuses = [STATUS.match(line) for line in lines if line.startswith("admission-v1")]
    completions = [COMPLETION.match(line) for line in lines if line.startswith("completion=")]
    timings = [ADMIT_TIMING.match(line) for line in lines if line.startswith("admit-v7")]
    if len(statuses) != 1 or not statuses[0] or not all(completions) or not all(timings):
        raise ValueError(f"incomplete or malformed admission status: {text!r}")
    if len(timings) != (1 if timing else 0):
        raise ValueError(f"admission timing present or missing: {text!r}")
    match = statuses[0]
    if match[2] != match[5]:
        raise ValueError("service instance names another lineage")
    result = {"id": match[1], "lineage": match[2], "number": int(match[3], 16), "instance": match[4],
              "instance_sequence": int(match[6], 16), "state": match[7], "terminal": int(match[8]),
              "lines": [line for line in lines if not line.startswith("admit-v7")]}
    expected = [f"completion=op_{result['lineage']}_{hex16(result['terminal'])}"] \
        if result["state"] == "committed" else []
    if [line for line in lines if line.startswith("completion=")] != expected:
        raise ValueError("the completion does not name the terminal transaction")
    result["completion"] = expected[0].split("=", 1)[1] if expected else None
    if (result["state"] == "admitted") != (result["terminal"] == 0):
        raise ValueError("an admitted status must have no terminal transaction and only it")
    if result["state"] != "admitted" and result["terminal"] <= result["number"]:
        raise ValueError("a terminal transaction must follow its admission")
    if timing:
        result["size"], result["ticks"] = int(timings[0][1]), int(timings[0][2])
    return result


def decode_observation(text):
    """Decode one retained `observe-admission-v2` answer."""
    lines = _lines(text, ("admission-observation", "completion=", "error"))
    heads = [OBSERVATION.match(line) for line in lines if line.startswith("admission-observation")]
    if len(heads) != 1 or not heads[0] or any(line.startswith("error") for line in lines):
        raise ValueError(f"incomplete or malformed observation: {text!r}")
    match = heads[0]
    result = {"id": match[1], "instance": match[2], "state": match[3], "terminal": int(match[4]),
              "prevention": match[5]}
    if (result["state"] == "cancelled") != (result["prevention"] != "none"):
        raise ValueError("a prevention cause must be present exactly for a cancelled admission")
    return result


def admission_record(snapshot, status):
    """The single retained record of the shell's subject for `status`'s admission."""
    records = [record for record in snapshot["records"]
               if record["subject"] == SHELL_SUBJECT and record["admission"] == status["number"]]
    if len(records) != 1:
        raise AssertionError(f"oracle7 finds {len(records)} records for admission {status['id']}")
    return records[0]


def check_record(snapshot, status, expected):
    """The oracle record must agree with the printed status and the request."""
    record = admission_record(snapshot, status)
    if snapshot["lineage"] != status["lineage"]:
        raise AssertionError("the admission names another lineage")
    want = {"state": ORACLE_STATES[status["state"]], "terminal": status["terminal"],
            "instance": status["instance_sequence"],
            "cause": "requested" if status["state"] == "cancelled" else None,
            "committed": status["terminal"] if status["state"] == "committed" else 0, **expected}
    for field, value in want.items():
        if record[field] != value:
            raise AssertionError(f"record {field}={record[field]!r}, expected {value!r}")
    if not record["runs"] and record["length"]:
        raise AssertionError("the staged snapshot owns no payload")
    return {key: record[key] for key in ("slot", "state", "admission", "terminal", "cause", "length", "sha256")}


def _file(snapshot, identity):
    return next(item for item in snapshot["files"] if item["id"] == identity)


def _live(data):
    return oracle7.snapshot(data.read_bytes())


def _admit_command(refs, version, epoch, key, seed, size):
    return command(refs["workspace"], refs["resource"], version, epoch, key, seed, size).replace(
        "replace-pattern-v7", "admit-pattern-v7", 1)


def _status(uart, text, timing=False):
    return decode_status(uart.command(text, "\n"), timing)


def _same(uart, commands, status, what):
    """Every command must print exactly `status`'s lines."""
    for text in commands:
        again = _status(uart, text, timing=text.startswith("admit-pattern-v7"))
        if again.get("lines") != status["lines"]:
            raise AssertionError(f"{what}: {text!r} printed {again}, expected {status['lines']}")


def _unchanged(data, before, what):
    if environment.digest(data) != before:
        raise AssertionError(f"{what} changed the image")


def _expected(refs_ids, epoch, key, previous, content):
    workspace, scratch = refs_ids
    return {"workspace": workspace, "object": scratch, "epoch": epoch, "key": key, "previous": previous,
            "length": len(content), "sha256": hashlib.sha256(content).hexdigest()}


def _first_boot(uart, data, refs, ids, state, evidence):
    content = pattern(LARGE_SEED, LARGE)
    before = _live(data)
    admit = _admit_command(refs, state["version"], state["epoch"], LARGE_KEY, LARGE_SEED, LARGE)
    started = time.monotonic()
    accepted = _status(uart, admit, timing=True)
    if accepted.get("state") != "admitted" or accepted["size"] != LARGE:
        raise AssertionError(f"the large admission was not admitted: {accepted}")
    accepted["host_seconds"] = round(time.monotonic() - started, 3)
    live = _live(data)
    record = check_record(live, accepted, _expected(ids, state["epoch"], LARGE_KEY, state["version"], content))
    if _file(live, ids[1]) != _file(before, ids[1]):
        raise AssertionError("admission changed the live file")
    if live["sequence"] != before["sequence"] + 1 or accepted["number"] != live["sequence"]:
        raise AssertionError("admission did not publish exactly one generation named by its ID")
    digest = environment.digest(data)
    _same(uart, [admit, f"admission-v7 {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(LARGE_KEY)}",
                 f"admission {accepted['id']}"], accepted, "admitted status")
    observed = decode_observation(uart.command(f"observe-admission-v2 {accepted['id']}", "\n"))
    if (observed["state"], observed["prevention"]) != ("admitted", "none"):
        raise AssertionError(f"observation of the pending admission: {observed}")
    maintained = decode_maintain(uart.command("maintain-v7", "\n"))
    if maintained != {"error": "Busy"}:
        raise AssertionError(f"maintenance with an unresolved admission was not Busy: {maintained}")
    _unchanged(data, digest, "the exact retry, status queries or refused maintenance")
    evidence.update({"accepted": accepted, "record": record, "oracle": view(live), "exact_retry": "identical",
                     "observation": observed, "maintenance": "Busy", "image_unchanged": True})
    return accepted


def _second_boot(uart, data, refs, ids, lineage, state, accepted, evidence):
    content = pattern(LARGE_SEED, LARGE)
    _same(uart, [f"admission {accepted['id']}"], accepted, "status after reboot")
    started = time.monotonic()
    executed = _status(uart, f"execute-admission {accepted['id']}")
    if executed.get("state") != "committed" or executed["number"] != accepted["number"] \
            or executed["instance"] != accepted["instance"]:
        raise AssertionError(f"execution did not commit the admission: {executed}")
    executed["host_seconds"] = round(time.monotonic() - started, 3)
    live = _live(data)
    record = check_record(live, executed, _expected(ids, state["epoch"], LARGE_KEY, state["version"], content))
    scratch = _file(live, ids[1])
    if (scratch["version"], scratch["sha256"]) != (executed["terminal"], hashlib.sha256(content).hexdigest()):
        raise AssertionError("the live file is not the executed admission")
    receipts = []
    for query in (executed["completion"], f"{refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(LARGE_KEY)}"):
        receipt = decode(uart.command(f"operation-v7 {query}", "\n"), LOOKUP_TIMING)
        if "error" in receipt:
            raise AssertionError(f"completion lookup {query!r} failed: {receipt['error']}")
        check_receipt(receipt, lineage, refs["workspace"], refs["resource"], state["version"], state["epoch"],
                      LARGE_KEY, content)
        if receipt["id"] != executed["completion"]:
            raise AssertionError("the receipt names another operation than the completion")
        receipts.append(receipt)
    if receipts[0]["lines"] != receipts[1]["lines"]:
        raise AssertionError("lookups by ID and retry key differ")
    digest = environment.digest(data)
    _same(uart, [f"execute-admission {accepted['id']}", f"cancel-admission {accepted['id']}",
                 _admit_command(refs, state["version"], state["epoch"], LARGE_KEY, LARGE_SEED, LARGE)],
          executed, "committed replay")
    _unchanged(data, digest, "committed replays")
    state["version"] = executed["terminal"]

    # A second admission overtaken by a concurrent tracked write.
    small = pattern(SMALL_SEED, SMALL)
    pending = _status(uart, _admit_command(refs, state["version"], state["epoch"], SMALL_KEY, SMALL_SEED, SMALL),
                      timing=True)
    if pending.get("state") != "admitted":
        raise AssertionError(f"the second admission was not admitted: {pending}")
    check_record(_live(data), pending, _expected(ids, state["epoch"], SMALL_KEY, state["version"], small))
    tracked_content = pattern(TRACKED_SEED, TRACKED)
    tracked = decode(uart.command(command(refs["workspace"], refs["resource"], state["version"], state["epoch"],
                                          TRACKED_KEY, TRACKED_SEED, TRACKED), "\n"))
    if "error" in tracked:
        raise AssertionError(f"the concurrent tracked write failed: {tracked['error']}")
    check_receipt(tracked, lineage, refs["workspace"], refs["resource"], state["version"], state["epoch"],
                  TRACKED_KEY, tracked_content)
    digest = environment.digest(data)
    refused = _status(uart, f"execute-admission {pending['id']}")
    if refused != {"error": "Version"}:
        raise AssertionError(f"execution after a concurrent write was not Version: {refused}")
    _same(uart, [f"admission {pending['id']}"], pending, "status after the refused execution")
    _unchanged(data, digest, "the refused execution")
    cancelled = _status(uart, f"cancel-admission {pending['id']}")
    if cancelled.get("state") != "cancelled" or cancelled["number"] != pending["number"]:
        raise AssertionError(f"the explicit cancel did not cancel: {cancelled}")
    observed = decode_observation(uart.command(f"observe-admission-v2 {pending['id']}", "\n"))
    if (observed["state"], observed["prevention"], observed["terminal"]) != \
            ("cancelled", "requested", cancelled["terminal"]):
        raise AssertionError(f"observation of the cancelled admission: {observed}")
    live = _live(data)
    cancelled_record = check_record(live, cancelled,
                                    _expected(ids, state["epoch"], SMALL_KEY, state["version"], small))
    scratch = _file(live, ids[1])
    if (scratch["version"], scratch["sha256"]) != (tracked["version"], tracked["sha256"]):
        raise AssertionError("the live file is not the concurrent tracked write")
    digest = environment.digest(data)
    _same(uart, [f"cancel-admission {pending['id']}", f"execute-admission {pending['id']}",
                 f"admission-v7 {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(SMALL_KEY)}"],
          cancelled, "cancelled replay")
    _unchanged(data, digest, "cancelled replays")
    state["version"] = tracked["version"]

    # Every admission is resolved: the owner's maintenance may now proceed.
    before = _live(data)
    maintained = decode_maintain(uart.command("maintain-v7", "\n"))
    if "error" in maintained:
        raise AssertionError(f"maintenance after resolution failed: {maintained['error']}")
    after = _live(data)
    check_maintained(before, after, maintained)
    state["epoch"] = maintained["epoch"]
    evidence.update({
        "status_after_reboot": "identical", "executed": executed, "executed_record": record,
        "completion_receipt": {key: receipts[0][key] for key in ("id", "version", "size", "sha256", "ticks")},
        "lookups_identical": True, "committed_replays": "identical", "pending": pending,
        "concurrent_write": {key: tracked[key] for key in ("id", "previous", "version", "size", "sha256")},
        "execute_after_write": "Version", "cancelled": cancelled, "cancelled_record": cancelled_record,
        "cancel_observation": observed, "cancelled_replays": "identical", "maintenance": maintained,
        "oracle_before_maintenance": view(before), "oracle_after_maintenance": view(after),
    })


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-admission")
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
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-admission-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-volume.raw"
            lineage = uuid.uuid4().hex
            seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
            scratch = seeded["scratch"]
            refs = {"workspace": seeded["workspace"]["text"], "resource": scratch["resource"]}
            ids = (seeded["workspace"]["id"], scratch["id"])
            initial = oracle7.snapshot(data.read_bytes())
            if len(initial["records"]) != SEEDED_RECORDS:
                raise AssertionError(f"seed7 provisioned {len(initial['records'])} records")
            state = {"epoch": initial["epoch"], "version": scratch["version"]}
            first, second = {}, {}
            accepted = boot_terminal(image, data, output, 1, temporary,
                                     lambda uart: _first_boot(uart, data, refs, ids, state, first))
            offline = oracle7.snapshot(data.read_bytes())
            check_record(offline, accepted, _expected(ids, state["epoch"], LARGE_KEY, state["version"],
                                                      pattern(LARGE_SEED, LARGE)))
            first["after_shutdown"] = view(offline)
            report = volume_json(volume_tool, "report7", data)
            if report["sequence"] != offline["sequence"]:
                raise AssertionError("report7 and oracle7 disagree after boot 1")
            boot_terminal(image, data, output, 2, temporary,
                          lambda uart: _second_boot(uart, data, refs, ids, lineage, state, accepted, second))
            final = oracle7.snapshot(data.read_bytes())
            if final["epoch"] != state["epoch"] or final["records"]:
                raise AssertionError("the maintained epoch or empty record table did not persist")
            if _file(final, ids[1])["version"] != state["version"]:
                raise AssertionError("the live file changed after the last phase")
            second["after_shutdown"] = view(final)
        evidence = {"verified": True, "mode": "terminal-v7", "boots": 2, "lineage": lineage,
                    "subject": SHELL_SUBJECT, "scratch": scratch, "initial": view(initial),
                    "boot_1": first, "boot_2": second}
        result = {"outcome": "success", "returncode": 33, "timed_out": False,
                  "elapsed_seconds": round(time.monotonic() - started, 3), "build_id": metadata["build_id"],
                  "image_sha256": metadata["image_sha256"], "terminal_v7_admission": evidence}
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print(f"V7 admission acceptance: a {LARGE}-byte admission stayed admitted across reboot with its staged "
              "snapshot (oracle7), exact retry identical, maintenance Busy; execution committed it and the "
              "completion receipt matched by ID and retry key; a concurrent write made execution Version with "
              "the record still admitted, an explicit cancel recorded the requested cause; replays wrote "
              "nothing; maintenance then dropped every record.", flush=True)
        print(f"V7 admission timing: admit_ticks={first['accepted']['ticks']} "
              f"execute_host_seconds={second['executed']['host_seconds']}", flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(
            b"\n".join((output / f"serial-{p}.log").read_bytes() for p in phases if (output / f"serial-{p}.log").exists()))
        (output / "qemu.log").write_bytes(
            b"\n".join((output / f"qemu-{p}.log").read_bytes() for p in phases if (output / f"qemu-{p}.log").exists()))
