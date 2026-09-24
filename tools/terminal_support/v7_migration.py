# SPDX-License-Identifier: Apache-2.0
"""Deliberate v5 -> v7 data migration, evidenced in disposable UEFI boots (#51).

A v5 volume retains at most two records, so `rustic-volume seed5-history`
builds three disposable v5 sources under one fresh lineage, and `migrate7`
converts each into a new V7 image; the sources are never given to QEMU and
their SHA-256 must not change. Seeded records belong to subject 2, the V7
shell's retry scope, except one deliberately left to subject 1.

* `receipts` (one boot): the migrated direct commit is looked up by operation
  ID and by retry key with the pattern's SHA-256, an exact replay prints the
  identical receipt, a mismatched retry is `IdempotencyConflict`, and the
  subject-1 record is `OutcomeUnknown` by ID and by retry key. The image digest
  does not change.
* `admissions` (two boots): the migrated admission reports `admitted` by ID,
  retry key and exact retry, a receipt lookup is `Busy`, `maintain-v7` is
  `Busy`; the migrated cancellation observes the `requested` cause and has no
  completed receipt (`Unsupported`); all with an unchanged image. Execution
  then commits the admission in exactly one publication (sequence + 1) into
  the other header slot, which also ends the migrated image's `recovered`
  report, and its completion receipt matches by ID and retry key. After a reboot every status and receipt line is identical, the
  image digest is unchanged, and `oracle7` finds the same generation.
* `completed` (one boot): the migrated executed admission reports `committed`
  with its completion, whose receipt matches by ID and retry key; exact
  admission, execute and cancel replays print the same status without writing.

`oracle7` checks each migrated image before its first boot, while the guest is
idle after a publishing step and after every shutdown. Executable rollback is
not part of this harness; it belongs to the launch harness (#52).
"""
import hashlib
import json
from pathlib import Path
import tempfile
import time
import uuid

import environment
from . import oracle7
from .v7_admission import decode_observation, decode_status
from .v7_read import volume_json
from .v7_retention import decode_maintain, view
from .v7_write import LOOKUP_TIMING, SHELL_SUBJECT, boot_terminal, check_receipt, command, decode, hex16, \
    lookup_error, pattern


ROOT = environment.ROOT
SETS = ("receipts", "admissions", "completed")
OWNER_SUBJECT = 1
EPOCH = 1


def admission_id(lineage, number):
    return f"ad_{lineage}_{hex16(number)}"


def operation_id(lineage, sequence):
    return f"op_{lineage}_{hex16(sequence)}"


def retry_query(workspace, key):
    return f"{workspace} e_{hex16(EPOCH)} k_{hex16(key)}"


def admit_command(workspace, record):
    return command(workspace, record["resource"], record["previous"], EPOCH, record["key"], record["seed"],
                   record["size"]).replace("replace-pattern-v7", "admit-pattern-v7", 1)


def content(record):
    return pattern(record["seed"], record["size"])


def check_migrated(seeded, migrated, snapshot, source_digest):
    """The migrated image must carry exactly the seeded history, and the source must be untouched."""
    if (migrated["source_sha256_before"], migrated["source_sha256_after"]) != (source_digest, source_digest):
        raise AssertionError(f"migrate7 {seeded['set']}: the v5 source digest changed")
    if (snapshot["lineage"], snapshot["sequence"], snapshot["epoch"]) != \
            (seeded["lineage"], seeded["sequence"], seeded["epoch"]):
        raise AssertionError(f"migrate7 {seeded['set']}: identity, sequence or epoch changed")
    if migrated["target"]["sequence"] != snapshot["sequence"]:
        raise AssertionError(f"migrate7 {seeded['set']}: report7 and oracle7 disagree")
    # Only generation 0 is written, at a non-initial sequence, so the mount
    # rule reports the missing other copy until the first V7 publication.
    if (snapshot["generation"], snapshot["recovered"], migrated["target"]["recovered"]) != (0, True, True):
        raise AssertionError(f"migrate7 {seeded['set']}: not a lone generation-0 header reported as recovered")
    fields = ("state", "cause", "subject", "object", "key", "previous", "committed", "admission", "terminal")
    ours = [tuple(record[field] for field in fields) + (record["sha256"], record["instance"], record["workspace"])
            for record in snapshot["records"]]
    wanted = [tuple(record[field] for field in fields)
              + (hashlib.sha256(content(record)).hexdigest(), seeded["instance"], seeded["workspace"]["id"])
              for record in seeded["records"]]
    if ours != wanted:
        raise AssertionError(f"migrate7 {seeded['set']}: records differ from the seed:\n{ours}\n!=\n{wanted}")


def _unchanged(data, before, what):
    if environment.digest(data) != before:
        raise AssertionError(f"{what} changed the image")


def _mounted(uart, phase):
    """Boot 1 is checked by `boot_terminal`; every other boot checks its own mount job."""
    if phase != 1:
        uart.command("job-status 1", "status=0")


def _record(seeded, state, subject=SHELL_SUBJECT):
    found = [record for record in seeded["records"] if record["state"] == state and record["subject"] == subject]
    if len(found) != 1:
        raise AssertionError(f"the {seeded['set']} seed has {len(found)} {state} records of subject {subject}")
    return found[0]


def _receipt(uart, query, seeded, record, lineage):
    """One `operation-v7` lookup that must name `record`'s committed version with its bytes."""
    receipt = decode(uart.command(f"operation-v7 {query}", "\n"), LOOKUP_TIMING)
    if "error" in receipt:
        raise AssertionError(f"lookup {query!r} failed: {receipt['error']}")
    check_receipt(receipt, lineage, seeded["workspace"]["text"], record["resource"], record["previous"], EPOCH,
                  record["key"], content(record))
    if receipt["version"] != record["committed"] or \
            receipt["service_instance"] != f"si_{lineage}_{hex16(seeded['instance'])}":
        raise AssertionError(f"lookup {query!r} names another version or service instance: {receipt}")
    return receipt


def _both_lookups(uart, seeded, record, lineage):
    """Lookups by operation ID and by retry key must print identical receipts."""
    by_id = _receipt(uart, operation_id(lineage, record["committed"]), seeded, record, lineage)
    by_key = _receipt(uart, retry_query(seeded["workspace"]["text"], record["key"]), seeded, record, lineage)
    if by_id["lines"] != by_key["lines"]:
        raise AssertionError("lookups by operation ID and retry key differ")
    return by_id


def _status(uart, text, timing=False):
    return decode_status(uart.command(text, "\n"), timing)


def _same(uart, commands, lines, what):
    for text in commands:
        again = _status(uart, text, timing=text.startswith("admit-pattern-v7"))
        if again.get("lines") != lines:
            raise AssertionError(f"{what}: {text!r} printed {again}, expected {lines}")


def _receipts_boot(uart, data, seeded, lineage, evidence):
    _mounted(uart, 1)
    workspace = seeded["workspace"]["text"]
    direct, foreign = _record(seeded, "direct_committed"), _record(seeded, "direct_committed", OWNER_SUBJECT)
    digest = environment.digest(data)
    receipt = _both_lookups(uart, seeded, direct, lineage)
    replay = decode(uart.command(command(workspace, direct["resource"], direct["previous"], EPOCH, direct["key"],
                                         direct["seed"], direct["size"]), "\n"))
    if replay.get("lines") != receipt["lines"]:
        raise AssertionError(f"the exact replay printed {replay}, expected {receipt['lines']}")
    conflict = decode(uart.command(command(workspace, direct["resource"], direct["previous"], EPOCH, direct["key"],
                                           direct["seed"] + 1, direct["size"]), "\n"))
    if conflict != {"error": "IdempotencyConflict"}:
        raise AssertionError(f"the mismatched retry was not an idempotency conflict: {conflict}")
    hidden = {
        "by_id": lookup_error(uart.command(f"operation-v7 {operation_id(lineage, foreign['committed'])}", "\n")),
        "by_retry_key": lookup_error(uart.command(f"operation-v7 {retry_query(workspace, foreign['key'])}", "\n")),
    }
    if set(hidden.values()) != {"OutcomeUnknown"}:
        raise AssertionError(f"the subject-1 record is visible to the V7 shell: {hidden}")
    _unchanged(data, digest, "lookups, the exact replay, the mismatched retry or the hidden lookups")
    evidence.update({"receipt": {key: receipt[key] for key in ("id", "service_instance", "previous", "version",
                                                                "size", "sha256", "ticks")},
                     "lookups_identical": True, "exact_replay": "identical", "mismatched_retry": conflict["error"],
                     "subject_1": hidden, "image_unchanged": True})


def _admissions_boot(uart, data, seeded, lineage, evidence):
    _mounted(uart, 2)
    workspace = seeded["workspace"]["text"]
    admitted, cancelled = _record(seeded, "admitted"), _record(seeded, "cancelled")
    pending_id, cancelled_id = admission_id(lineage, admitted["admission"]), admission_id(lineage, cancelled["admission"])
    digest = environment.digest(data)
    before = oracle7.snapshot(data.read_bytes())

    pending = _status(uart, f"admission {pending_id}")
    if (pending.get("state"), pending.get("number"), pending.get("instance_sequence")) != \
            ("admitted", admitted["admission"], seeded["instance"]):
        raise AssertionError(f"the migrated admission is not admitted: {pending}")
    _same(uart, [f"admission-v7 {retry_query(workspace, admitted['key'])}", admit_command(workspace, admitted)],
          pending["lines"], "migrated admission status")
    busy = lookup_error(uart.command(f"operation-v7 {retry_query(workspace, admitted['key'])}", "\n"))
    observed = decode_observation(uart.command(f"observe-admission-v2 {pending_id}", "\n"))
    if (busy, observed["state"], observed["prevention"]) != ("Busy", "admitted", "none"):
        raise AssertionError(f"the migrated admission lookup {busy} or observation {observed}")
    stopped = _status(uart, f"admission {cancelled_id}")
    if (stopped.get("state"), stopped.get("terminal")) != ("cancelled", cancelled["terminal"]):
        raise AssertionError(f"the migrated cancellation is not cancelled: {stopped}")
    cause = decode_observation(uart.command(f"observe-admission-v2 {cancelled_id}", "\n"))
    if (cause["state"], cause["prevention"], cause["terminal"]) != ("cancelled", "requested", cancelled["terminal"]):
        raise AssertionError(f"the migrated cancellation cause is not requested: {cause}")
    no_receipt = lookup_error(uart.command(f"operation-v7 {retry_query(workspace, cancelled['key'])}", "\n"))
    if no_receipt != "Unsupported":
        raise AssertionError(f"a cancelled admission produced a receipt answer: {no_receipt}")
    maintained = decode_maintain(uart.command("maintain-v7", "\n"))
    if maintained != {"error": "Busy"}:
        raise AssertionError(f"maintenance with a migrated unresolved admission was not Busy: {maintained}")
    _unchanged(data, digest, "status queries, the exact retry, the refused lookup or the refused maintenance")

    started = time.monotonic()
    executed = _status(uart, f"execute-admission {pending_id}")
    if (executed.get("state"), executed.get("number")) != ("committed", admitted["admission"]):
        raise AssertionError(f"execution did not commit the migrated admission: {executed}")
    executed["host_seconds"] = round(time.monotonic() - started, 3)
    after = oracle7.snapshot(data.read_bytes())
    record = next(item for item in after["records"] if item["admission"] == admitted["admission"])
    live = next(item for item in after["files"] if item["id"] == admitted["object"])
    if (record["state"], record["committed"], record["terminal"]) != \
            ("admitted_committed", executed["terminal"], executed["terminal"]) \
            or (live["version"], live["sha256"]) != (executed["terminal"], hashlib.sha256(content(admitted)).hexdigest()) \
            or after["sequence"] != before["sequence"] + 1:
        raise AssertionError("execution did not publish exactly one generation committing the migrated bytes")
    if (after["generation"], after["recovered"]) != (1 - before["generation"], False):
        raise AssertionError("the first V7 publication did not fill the other header slot and clear recovery")
    committed = dict(admitted, committed=executed["terminal"])
    receipt = _both_lookups(uart, seeded, committed, lineage)
    if receipt["id"] != executed["completion"]:
        raise AssertionError("the completion receipt names another operation")
    digest = environment.digest(data)
    _same(uart, [f"execute-admission {pending_id}", f"cancel-admission {pending_id}"], executed["lines"],
          "committed replay")
    _unchanged(data, digest, "committed replays")
    evidence.update({"admitted": pending, "lookup_while_admitted": busy, "observation": observed,
                     "cancelled": stopped, "cancel_observation": cause, "cancelled_receipt": no_receipt,
                     "maintenance": maintained["error"], "executed": executed,
                     "completion_receipt": {key: receipt[key] for key in ("id", "previous", "version", "size",
                                                                          "sha256", "ticks")},
                     "oracle_before": dict(view(before), generation=before["generation"],
                                           recovered=before["recovered"]),
                     "oracle_after": dict(view(after), generation=after["generation"],
                                          recovered=after["recovered"])})
    return {"executed": executed["lines"], "cancelled": stopped["lines"], "receipt": receipt["lines"],
            "cause": cause, "ids": (pending_id, cancelled_id), "committed": committed}


def _admissions_reboot(uart, data, seeded, lineage, printed, evidence):
    _mounted(uart, 3)
    workspace = seeded["workspace"]["text"]
    pending_id, cancelled_id = printed["ids"]
    digest = environment.digest(data)
    _same(uart, [f"admission {pending_id}"], printed["executed"], "committed status after reboot")
    _same(uart, [f"admission {cancelled_id}"], printed["cancelled"], "cancelled status after reboot")
    cause = decode_observation(uart.command(f"observe-admission-v2 {cancelled_id}", "\n"))
    if cause != printed["cause"]:
        raise AssertionError(f"the cancellation cause changed across reboot: {cause}")
    committed = printed["committed"]
    receipt = _both_lookups(uart, seeded, committed, lineage)
    if receipt["lines"] != printed["receipt"]:
        raise AssertionError("the completion receipt changed across reboot")
    again = decode(uart.command(f"operation-v7 {retry_query(workspace, committed['key'])}", "\n"), LOOKUP_TIMING)
    if again.get("lines") != printed["receipt"]:
        raise AssertionError("a repeated retry-key lookup printed another receipt")
    _unchanged(data, digest, "the rebooted queries")
    evidence.update({"statuses_identical": True, "receipt_identical": True, "cause": cause["prevention"],
                     "image_unchanged": True})


def _completed_boot(uart, data, seeded, lineage, evidence):
    _mounted(uart, 4)
    workspace = seeded["workspace"]["text"]
    done = _record(seeded, "admitted_committed")
    done_id = admission_id(lineage, done["admission"])
    digest = environment.digest(data)
    status = _status(uart, f"admission {done_id}")
    if (status.get("state"), status.get("terminal"), status.get("completion")) != \
            ("committed", done["terminal"], operation_id(lineage, done["committed"])):
        raise AssertionError(f"the migrated executed admission is not committed: {status}")
    receipt = _both_lookups(uart, seeded, done, lineage)
    _same(uart, [admit_command(workspace, done), f"admission-v7 {retry_query(workspace, done['key'])}",
                 f"execute-admission {done_id}", f"cancel-admission {done_id}"], status["lines"],
          "migrated committed replay")
    _unchanged(data, digest, "the committed status, lookups or replays")
    evidence.update({"status": status, "receipt": {key: receipt[key] for key in ("id", "previous", "version", "size",
                                                                                  "sha256", "ticks")},
                     "replays": "identical", "image_unchanged": True})


def _prepare(volume_tool, temporary, lineage, name):
    source, target = temporary / f"{name}.v5", temporary / f"{name}.v7"
    seeded = volume_json(volume_tool, "seed5-history", source, lineage, name)
    source_digest = environment.digest(source)
    migrated = volume_json(volume_tool, "migrate7", source, target, lineage)
    snapshot = oracle7.snapshot(target.read_bytes())
    check_migrated(seeded, migrated, snapshot, source_digest)
    return {"source": source, "target": target, "seeded": seeded, "source_sha256": source_digest,
            "initial": snapshot}


def _offline(entry, expected=None):
    """The shut-down image must be what the guest left (or `expected`)."""
    snapshot = oracle7.snapshot(entry["target"].read_bytes())
    reference = expected or entry["initial"]
    for key in ("lineage", "generation", "recovered", "sequence", "epoch", "next", "files", "records",
                "free_sectors"):
        if snapshot[key] != reference[key]:
            raise AssertionError(f"{entry['seeded']['set']}: oracle7 {key} changed while the guest was down")
    return snapshot


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-migration")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    (output / "result.json").unlink(missing_ok=True)
    started = time.monotonic()
    phases = [1, 2, 3, 4]
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-migration-") as temporary:
            temporary = Path(temporary)
            lineage = uuid.uuid4().hex
            sets = {name: _prepare(volume_tool, temporary, lineage, name) for name in SETS}
            receipts, admissions, completed = (sets[name] for name in SETS)
            evidence = {name: {"seed": entry["seeded"], "source_sha256": entry["source_sha256"],
                               "initial": view(entry["initial"]), "recovered": entry["initial"]["recovered"]}
                        for name, entry in sets.items()}

            boot_terminal(image, receipts["target"], output, 1, temporary,
                          lambda uart: _receipts_boot(uart, receipts["target"], receipts["seeded"], lineage,
                                                      evidence["receipts"]))
            _offline(receipts)

            printed = boot_terminal(image, admissions["target"], output, 2, temporary,
                                    lambda uart: _admissions_boot(uart, admissions["target"], admissions["seeded"],
                                                                  lineage, evidence["admissions"]))
            executed = oracle7.snapshot(admissions["target"].read_bytes())
            boot_terminal(image, admissions["target"], output, 3, temporary,
                          lambda uart: _admissions_reboot(uart, admissions["target"], admissions["seeded"], lineage,
                                                          printed, evidence["admissions"].setdefault("reboot", {})))
            evidence["admissions"]["after_reboot"] = view(_offline(admissions, executed))

            boot_terminal(image, completed["target"], output, 4, temporary,
                          lambda uart: _completed_boot(uart, completed["target"], completed["seeded"], lineage,
                                                       evidence["completed"]))
            _offline(completed)

            for name, entry in sets.items():
                if environment.digest(entry["source"]) != entry["source_sha256"]:
                    raise AssertionError(f"the {name} v5 source changed")
                evidence[name]["source_unchanged"] = True
        result = {"outcome": "success", "returncode": 33, "timed_out": False,
                  "elapsed_seconds": round(time.monotonic() - started, 3), "build_id": metadata["build_id"],
                  "image_sha256": metadata["image_sha256"],
                  "terminal_v7_migration": {"verified": True, "mode": "terminal-v7", "boots": len(phases),
                                            "lineage": lineage, "shell_subject": SHELL_SUBJECT,
                                            "sets": evidence}}
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print("V7 migration acceptance: three seeded v5 histories migrated with unchanged source digests; the "
              "migrated direct commit matched by ID and retry key, replayed identically and refused a mismatched "
              "retry; the subject-1 record stayed hidden; the migrated admission was Busy to lookup and "
              "maintenance, executed and survived reboot with identical receipts; the cancellation kept its "
              "requested cause; the executed admission replayed without writing.", flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(
            b"\n".join((output / f"serial-{p}.log").read_bytes() for p in phases if (output / f"serial-{p}.log").exists()))
        (output / "qemu.log").write_bytes(
            b"\n".join((output / f"qemu-{p}.log").read_bytes() for p in phases if (output / f"qemu-{p}.log").exists()))
