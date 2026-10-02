# SPDX-License-Identifier: Apache-2.0
"""Verify TASKS_OWNER recovery of a committed V7 effect after a lost reply."""
import hashlib
import json
from pathlib import Path
import tempfile
import time
import uuid

import environment
from . import oracle7, tasks_owner
from .authority_cases import cleanup
from .cases import counters, pid
from .v7_plain import metadata
from .v7_read import volume_json
from .v7_write import boot_terminal


ROOT = environment.ROOT
POLICY = b"rustic-owner-v1\nhelpers=explicit\n"
DOCUMENT = tasks_owner.DOCUMENT
TITLE = "Recover V7 reply"
EDITED = tasks_owner.added(DOCUMENT, TITLE)
DOCUMENT_PATH = "/workspaces/tasks-recovery-doc"
JOURNAL_PATH = "/workspaces/tasks-recovery-journal"


def snapshot(data):
    return oracle7.snapshot(data.read_bytes())


def _intent_state(state, document, journal, candidate, task):
    """Check the retained intent, target and receipt independently on disk."""
    expected = candidate.encode("ascii")
    contents = state["contents"]
    if contents.get(DOCUMENT_PATH) != expected:
        raise AssertionError("V7 target bytes differ from the exact handed candidate")
    journal_bytes = contents.get(JOURNAL_PATH, b"")
    if not journal_bytes.startswith(tasks_owner.MAGIC) or len(journal_bytes) <= 136:
        raise AssertionError("lost reply did not leave a nonempty owner journal")
    if len(state["records"]) != 1:
        raise AssertionError(f"expected one retained TASKS_OWNER receipt, found {len(state['records'])}")
    record = state["records"][0]
    node_document = state["nodes"][document["id"]]
    node_journal = state["nodes"][journal["id"]]
    expected_digest = hashlib.sha256(expected).hexdigest()
    if (record["subject"], record["workspace"], record["object"], record["state"],
            record["key"], record["previous"], record["committed"], record["terminal"],
            record["length"], record["sha256"], record["target"]) != (
            journal["id"], 4, document["id"], "direct_committed", node_journal["version"],
            document["version"], node_document["version"], node_document["version"],
            len(expected), expected_digest,
            {"path": DOCUMENT_PATH, "version": node_document["version"]}):
        raise AssertionError(f"retained receipt does not describe the target and its journal: {record}")
    if task != 43 or not expected.endswith(f"{task}\topen\t{TITLE}\n".encode("ascii")):
        raise AssertionError("handed task identifier or candidate differs from the planned task")
    return {"journal_bytes": len(journal_bytes), "journal_key": record["key"],
            "receipt": record, "target_version": node_document["version"],
            "target_sha256": expected_digest}


def _first_boot(uart, data):
    baseline = counters(uart)
    tasks_owner.write_document(uart, DOCUMENT_PATH, DOCUMENT)
    uart.command(f"touch {JOURNAL_PATH}")
    document = metadata(uart, DOCUMENT_PATH)
    journal = metadata(uart, JOURNAL_PATH)
    if (document["parent"], journal["parent"], document["kind"], journal["kind"]) != (
            4, 4, "file", "file") or document["id"] == journal["id"]:
        raise AssertionError("task document and journal are not distinct workspace files")
    initial_digest = environment.digest(data)
    child = pid(uart, f"tasks-owner {DOCUMENT_PATH} {JOURNAL_PATH}")
    task = tasks_owner.hand(uart, child, "add", f'"{TITLE}"', EDITED, path=DOCUMENT_PATH)
    if task != 43:
        raise AssertionError(f"planned task identifier is {task}, expected 43")
    if environment.digest(data) != initial_digest:
        raise AssertionError("planning the candidate changed the V7 volume")

    lost = tasks_owner.apply_cut(uart, child, tasks_owner.LOST_REPLY, tasks_owner.UNCERTAIN)
    committed_document = metadata(uart, DOCUMENT_PATH)
    pending_journal = metadata(uart, JOURNAL_PATH)
    expected_reply = {"error": tasks_owner.UNCERTAIN, "task": 0,
                      "journal": pending_journal["version"],
                      "applied": 0, "version": 0}
    if lost != expected_reply:
        raise AssertionError(f"lost-reply cut returned {lost}, expected {expected_reply}")
    cleanup(uart, child)
    if counters(uart) != baseline:
        raise AssertionError("first TASKS_OWNER process resources were not reclaimed")
    return {"document": document, "journal": journal, "committed_document": committed_document,
            "pending_journal": pending_journal, "task": task, "apply": lost}


def _second_boot(uart, data, first, prior_state, receipt_facts):
    baseline = counters(uart)
    document = metadata(uart, DOCUMENT_PATH)
    journal = metadata(uart, JOURNAL_PATH)
    if document != first["committed_document"] or journal != first["pending_journal"]:
        raise AssertionError("reboot changed the document or pending journal metadata")
    if snapshot(data) != prior_state:
        raise AssertionError("second-boot observation changed the pending V7 state")

    child = pid(uart, f"tasks-owner {DOCUMENT_PATH} {JOURNAL_PATH}")
    pending = tasks_owner.query(uart, child)
    key = receipt_facts["journal_key"]
    if pending != {"error": 0, "phase": tasks_owner.IDLE, "cursor": 0,
                   "total": 0, "pending": key}:
        raise AssertionError(f"fresh TASKS_OWNER did not bind the pending journal: {pending}")

    recovered = tasks_owner.recover(uart, child)
    expected_recovery = {"error": 0, "recovered": 1, "journal": key,
                         "task": first["task"], "version": receipt_facts["target_version"]}
    if recovered != expected_recovery:
        raise AssertionError(f"journal recovery returned {recovered}, expected {expected_recovery}")
    after = snapshot(data)
    expected = EDITED.encode("ascii")
    if after["contents"].get(DOCUMENT_PATH) != expected:
        raise AssertionError("recovery changed the committed document bytes")
    if after["nodes"][document["id"]]["version"] != receipt_facts["target_version"]:
        raise AssertionError("recovery resubmitted or changed the committed target version")
    if after["contents"].get(JOURNAL_PATH) != b"":
        raise AssertionError("successful recovery did not clear the retained journal")
    cleared_journal = after["nodes"][journal["id"]]
    if cleared_journal["version"] <= journal["version"]:
        raise AssertionError("clearing the journal did not advance its file version")
    if len(after["records"]) != 1 or after["records"][0] != receipt_facts["receipt"]:
        raise AssertionError("recovery changed or replaced the exact retained target receipt")

    after_recovery_digest = environment.digest(data)
    idle = tasks_owner.recover(uart, child)
    expected_idle = {"error": 0, "recovered": 0, "journal": 0, "task": 0, "version": 0}
    if idle != expected_idle:
        raise AssertionError(f"idle recovery returned {idle}, expected {expected_idle}")
    if environment.digest(data) != after_recovery_digest or snapshot(data) != after:
        raise AssertionError("idle repeat recovery changed the V7 volume")
    uart.command(f"tasks list {DOCUMENT_PATH}", "3 tasks")
    cleanup(uart, child)
    if counters(uart) != baseline:
        raise AssertionError("second TASKS_OWNER process resources were not reclaimed")
    return {"pending": pending, "recover": recovered, "idle_repeat": idle,
            "target_version": receipt_facts["target_version"],
            "journal_version_after_clear": cleared_journal["version"],
            "receipt_unchanged": True, "idle_volume_unchanged": True}


def verify(image, volume_tool, output=None):
    image, volume_tool = Path(image).resolve(), Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-tasks-recovery")
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").unlink(missing_ok=True)
    identity = json.loads((image.parent / "image.json").read_text())
    if identity.get("mode") != "terminal-v7" or identity.get("tasks_acceptance") is not True:
        raise RuntimeError("TASKS_OWNER lost-reply recovery requires terminal-v7 with tasks acceptance enabled")
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="rustic-v7-tasks-recovery-") as temporary:
        temporary = Path(temporary)
        data = temporary / "owner.raw"
        lineage = uuid.uuid4().hex
        volume_json(volume_tool, "provision7", data, lineage)
        initial = snapshot(data)
        if initial["contents"].get("/config/owner-policy") != POLICY or initial["records"]:
            raise AssertionError("fresh V7 owner-policy fixture is invalid or already has receipts")

        first = boot_terminal(image, data, output, 1, temporary, lambda uart: _first_boot(uart, data))
        pending = snapshot(data)
        facts = _intent_state(pending, first["document"], first["journal"], EDITED, first["task"])
        second = boot_terminal(image, data, output, 2, temporary,
                               lambda uart: _second_boot(uart, data, first, pending, facts))
        final = snapshot(data)
        if final["contents"].get(DOCUMENT_PATH) != EDITED.encode("ascii"):
            raise AssertionError("final V7 oracle read differs from the recovered target")
        if final["contents"].get(JOURNAL_PATH) != b"" or final["records"] != [facts["receipt"]]:
            raise AssertionError("final V7 volume lost the cleared journal or exact receipt")
        if volume_json(volume_tool, "report7", data)["sequence"] != final["sequence"]:
            raise AssertionError("Rust remount and independent V7 oracle disagree")

    result = {"outcome": "success", "returncode": 33, "timed_out": False,
              "tasks_acceptance": identity["tasks_acceptance"],
              "build_id": identity["build_id"], "image_sha256": identity["image_sha256"],
              "elapsed_seconds": round(time.monotonic() - started, 3),
              "terminal_v7_tasks_recovery": {"verified": True, "boots": 2, "lineage": lineage,
                                             "first": first, "pending": {
                                                 "journal_bytes": facts["journal_bytes"],
                                                 "journal_key": facts["journal_key"],
                                                 "receipt": facts["receipt"],
                                                 "target_version": facts["target_version"],
                                                 "target_sha256": facts["target_sha256"]},
                                             "second": second}}
    (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
    print("V7 TASKS_OWNER lost-reply recovery: nonempty intent, exact committed receipt, no target resubmission, and idle repeat verified.",
          flush=True)
    return result
