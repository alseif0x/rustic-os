# SPDX-License-Identifier: Apache-2.0
"""Existing embedded native apps, explicit grants and V7 persistent outcomes."""
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import time
import uuid

import environment
from fs7_test import node_offset, reseal_header, reseal_node
from . import oracle7, tasks_owner
from .authority_cases import actor, actor_result, cleanup, fence
from .cases import counters, exited, pid
from .v7_plain import metadata
from .v7_read import volume_json
from .v7_write import boot_terminal

ROOT = environment.ROOT
POLICY = b"rustic-owner-v1\nhelpers=explicit\n"
DOCUMENT = tasks_owner.DOCUMENT
EDITED = tasks_owner.added(DOCUMENT, "Verify V7")
CLOSED = 23  # crates/abi/src/files.rs: Error::Closed


def snapshot(data):
    return oracle7.snapshot(data.read_bytes())


def invalid_policy(image, volume_tool, output, temporary, original):
    """A mountable same-length policy mismatch, not corrupt storage media."""
    data = temporary / "invalid-policy.raw"
    raw = bytearray(original.read_bytes())
    state = oracle7.snapshot(raw)
    policy = next(node for node in state["nodes"].values() if node["path"] == "/config/owner-policy")
    assert policy["runs"][0][1] == 1 and policy["length"] == len(POLICY)
    offset = (oracle7.PAYLOAD_SECTOR + policy["runs"][0][0]) * oracle7.SECTOR
    malformed = b"X" + POLICY[1:]
    raw[offset:offset + len(malformed)] = malformed
    at = node_offset(raw, state["generation"], policy["id"])
    struct.pack_into("<I", raw, at + 120, oracle7.crc32(malformed))
    reseal_node(raw, at)
    reseal_header(raw, state["generation"])
    data.write_bytes(raw)
    assert snapshot(data)["contents"]["/config/owner-policy"] == malformed
    assert volume_json(volume_tool, "report7", data)["sequence"] == state["sequence"]
    before = environment.digest(data)

    def check(uart):
        baseline = counters(uart)
        uart.command("ls /workspaces")
        uart.command("run read /workspaces", "error: service denied")
        uart.command("restart files", "utility sessions revoked")
        uart.command("run read /workspaces", "error: service denied")
        assert counters(uart) == baseline
        return {"same_length_mismatch_denied": True, "mount_and_restart_usable": True}

    result = boot_terminal(image, data, output, 3, temporary, check)
    assert environment.digest(data) == before
    return {**result, "volume_unchanged": True}


def authority(uart):
    baseline = counters(uart)
    uart.command("write authority-a before")
    uart.command("write authority-b untouched")
    root = pid(uart, "session authority-a authority-b")
    before = counters(uart)
    uart.command(f"helper {root} authority-b authority-a", "denied")
    assert counters(uart) == before
    helper = pid(uart, f"helper {root} authority-a authority-b")
    uart.command(f"helper {helper} authority-a authority-b", "denied")
    for child in (root, helper):
        observed = actor(uart, child, "read")
        assert (observed["value"], observed["other"], observed["control_denied"]) == (6, 17, 1)
    actor(uart, helper, "stage", 17)
    actor(uart, root, "stage")
    fills = [actor(uart, child, "flood") for child in (root, helper)]
    assert all(result["value"] >= 4 and result["other"] > 0 for result in fills), fills
    uart.command("write authority-a owner-under-pressure")
    settlement = fence(uart, helper, "access=fenced members=2 discarded_staging=1 effects=settled")
    drains = []
    for child in (root, helper):
        # V7 retires the entire endpoint, so drain observes channel closure;
        # queued requests/replies cannot be consumed by a fresh binding.
        drains.append(actor(uart, child, "drain", 1))
        actor(uart, child, "read", CLOSED)
        uart.command(f"permissions {child}", "rights=0")
    # The SDK maps a lost COMMIT exchange to Uncertain even on a closed channel.
    # The owner's settled revocation and unchanged bytes establish prevention.
    actor(uart, root, "commit", 3)
    uart.command("cat authority-a", "owner-under-pressure")
    cleanup(uart, root, helper)
    assert counters(uart) == baseline

    fresh = pid(uart, "session authority-a authority-b")
    helper = pid(uart, f"helper {fresh} authority-a authority-b")
    actor(uart, fresh, "read")
    actor(uart, fresh, "stage")
    actor(uart, fresh, "commit")
    assert actor(uart, helper, "read")["value"] == 19
    uart.command("cat authority-a", "session client edit")
    cleanup(uart, fresh, helper)
    assert counters(uart) == baseline
    return {"queued_pressure": fills, "drains": drains, "settlement": settlement.strip(),
            "old_endpoint_status": CLOSED, "fresh_group_committed": True,
            "resources_reclaimed": True}


def tasks(uart, data):
    baseline = counters(uart)
    tasks_owner.write_document(uart, "tasks-doc", DOCUMENT)
    # Same basename as the shell's private /config record is valid here.
    uart.command("touch tasks-intent")
    doc = metadata(uart, "tasks-doc")
    journal = metadata(uart, "tasks-intent")
    uart.command("tasks-owner tasks-doc tasks-doc", "error: invalid arguments; type help")
    uart.command("tasks-owner tasks-doc /workspaces", "error: service denied")
    assert counters(uart) == baseline
    digest = environment.digest(data)
    uart.command("tasks list tasks-doc", "2 tasks")
    assert environment.digest(data) == digest
    child = pid(uart, "tasks-owner tasks-doc tasks-intent")
    task = tasks_owner.hand(uart, child, "add", '"Verify V7"', EDITED, path="tasks-doc")
    result = tasks_owner.apply(uart, child)
    assert result["applied"] == 1 and result["task"] == task == 43
    state = snapshot(data)
    assert state["contents"]["/workspaces/tasks-doc"] == EDITED.encode("ascii")
    assert state["contents"]["/workspaces/tasks-intent"] == b""
    records = [r for r in state["records"] if r["subject"] == journal["id"]]
    assert len(records) == 1
    record = records[0]
    assert (record["workspace"], record["object"], record["previous"], record["committed"],
            record["key"], record["sha256"]) == (
                4, doc["id"], doc["version"], result["version"], result["journal"],
                hashlib.sha256(EDITED.encode("ascii")).hexdigest())
    cleanup(uart, child)
    successor = pid(uart, "tasks-owner tasks-doc tasks-intent")
    digest = environment.digest(data)
    recovered = tasks_owner.recover(uart, successor)
    assert recovered["recovered"] == 0 and environment.digest(data) == digest
    cleanup(uart, successor)
    assert counters(uart) == baseline
    return {"document": doc["id"], "journal_subject": journal["id"], "apply": result,
            "idle_recovery": recovered, "resources_reclaimed": True}


def first_boot(uart, data):
    baseline = counters(uart)
    uart.command("write probe-a native-read")
    uart.command("write probe-b private")
    child = pid(uart, "run probe probe-a probe-b")
    exited(uart, child, 1, 0)
    uart.command(f"permissions {child}", "report=0 bytes=11 other=17")
    uart.command(f"reap {child}", "code=0")
    assert counters(uart) == baseline
    group = authority(uart)
    task = tasks(uart, data)
    uart.command("write lost-target before")
    child = pid(uart, "run lost-admission lost-target probe-b")
    exited(uart, child, 1, 0)
    uart.command(f"reap {child}", "code=0")
    state = snapshot(data)
    records = [r for r in state["records"] if r["subject"] == 1]
    assert len(records) == 1 and records[0]["state"] == "admitted"
    record = records[0]
    assert state["contents"]["/workspaces/lost-target"] == b"before"
    admission = f'ad_{state["lineage"]}_{record["admission"]:016x}'
    # Shell subject2 cannot claim the subject1 consumer's lost acceptance.
    uart.command(f"admission {admission}", "error: OutcomeUnknown")
    assert counters(uart) == baseline
    return {"probe": True, "authority": group, "tasks": task, "admission": admission,
            "record": {key: record[key] for key in ("subject", "object", "admission", "previous")}}


def second_boot(uart, data, first):
    baseline = counters(uart)
    before = environment.digest(data)
    child = pid(uart, "admission-session lost-target probe-b 7")
    admission = first["admission"]
    uart.command(f"act-admission {child} get {admission}", "actor state=pending")
    pending = actor_result(uart, child)
    assert pending["value"] == 1 and environment.digest(data) == before
    uart.command("cat lost-target", "before")
    uart.command(f"act-admission {child} execute {admission}", "actor state=pending")
    complete = actor_result(uart, child)
    assert complete["value"] == 3 and complete["other"] > first["record"]["admission"]
    uart.command("cat lost-target", "reply deliberately unobserved")
    after = environment.digest(data)
    uart.command(f"act-admission {child} execute {admission}", "actor state=pending")
    assert actor_result(uart, child) == complete and environment.digest(data) == after
    cleanup(uart, child)
    uart.command("restart files", "utility sessions revoked")
    uart.command("tasks list tasks-doc", "3 tasks")
    assert environment.digest(data) == after and counters(uart) == baseline
    return {"pending_after_reboot": pending, "explicit_execution": complete,
            "replay_unchanged": True, "read_only_restart_unchanged": True}


def verify(image, volume_tool, output=None):
    image, volume_tool = Path(image).resolve(), Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-consumers")
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").unlink(missing_ok=True)
    identity = json.loads((image.parent / "image.json").read_text())
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="rustic-v7-consumers-") as temporary:
        temporary = Path(temporary)
        data = temporary / "owner.raw"
        lineage = uuid.uuid4().hex
        volume_json(volume_tool, "provision7", data, lineage)
        initial = snapshot(data)
        assert initial["contents"]["/config/owner-policy"] == POLICY and not initial["records"]
        invalid = invalid_policy(image, volume_tool, output, temporary, data)
        first = boot_terminal(image, data, output, 1, temporary, lambda uart: first_boot(uart, data))
        second = boot_terminal(image, data, output, 2, temporary, lambda uart: second_boot(uart, data, first))
        final = snapshot(data)
        assert final["contents"]["/config/owner-policy"] == POLICY
        assert final["contents"]["/workspaces/tasks-doc"] == EDITED.encode("ascii")
        assert final["contents"]["/workspaces/authority-a"] == b"session client edit"
        assert final["contents"]["/workspaces/authority-b"] == b"untouched"
        record = next(r for r in final["records"] if r["subject"] == 1)
        assert len(final["records"]) == 2 and record["state"] == "admitted_committed"
        assert record["committed"] == second["explicit_execution"]["other"]
        assert record["sha256"] == hashlib.sha256(b"reply deliberately unobserved").hexdigest()
        assert volume_json(volume_tool, "report7", data)["sequence"] == final["sequence"]
    result = {"outcome": "success", "returncode": 33, "timed_out": False,
              "build_id": identity["build_id"], "image_sha256": identity["image_sha256"],
              "elapsed_seconds": round(time.monotonic() - started, 3),
              "terminal_v7_consumers": {"verified": True, "boots": 3, "lineage": lineage,
                                         "invalid_policy": invalid,
                                         "first": first, "second": second}}
    (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
    print("V7 native consumers: scoped reads, tasks owner, queued helper revocation, fresh grants, "
          "lost acceptance across reboot and explicit execution verified.", flush=True)
    return result
