# SPDX-License-Identifier: Apache-2.0
"""Original owner commands, shared V7 retention, and manual service recovery."""
import json
from pathlib import Path
import re
import tempfile
import time
import uuid

import environment
from . import oracle7
from .admission_cases import status
from .authority_cases import actor, actor_result, cleanup, fence
from .cases import counters, exited, pid
from .operation_cases import references
from .v7_plain import metadata
from .v7_read import volume_json
from .v7_retention import check_maintained, reclaimable
from .v7_write import boot_terminal

ROOT = environment.ROOT
ENABLE = ("enable-operations", "enable-admissions", "enable-prevention-reasons")


def snapshot(data):
    return oracle7.snapshot(data.read_bytes())


def enable(uart, data, *, busy=False):
    before = environment.digest(data)
    replies = []
    for command in ENABLE:
        replies.append(uart.command(command, "error: Busy" if busy else "mounted format v7"))
    assert environment.digest(data) == before
    return {"commands": len(replies), "writes": False, "busy": busy}


def rotate(uart, data):
    before = snapshot(data)
    text = uart.command("rotate-receipts", "previous receipts expired")
    epochs = re.findall(r"retry epoch=(\d+) previous receipts expired", text)
    assert len(epochs) == 1
    after = snapshot(data)
    report = {"previous": before["epoch"], "epoch": int(epochs[0]),
              "records": len(before["records"]), "sectors": reclaimable(before)}
    check_maintained(before, after, report)
    return report


def maintenance(uart, data):
    baseline = counters(uart)
    confirmed = enable(uart, data)
    uart.command("write owner-target before")
    uart.command("write owner-other private")
    root = pid(uart, "session owner-target owner-other")
    actor(uart, root, "read")
    actor(uart, root, "stage")
    busy = enable(uart, data, busy=True)
    digest = environment.digest(data)
    uart.command("rotate-receipts", "error: Busy")
    assert environment.digest(data) == digest
    actor(uart, root, "commit")
    cleanup(uart, root)

    workspace, resource = references(uart, "/workspaces", "owner-target")
    target = metadata(uart, "owner-target")
    epoch = snapshot(data)["epoch"]
    accepted = status(uart.command(
        f'admit-ref {workspace} {resource} v_{target["version"]:016x} '
        f'e_{epoch:016x} k_0000000000000077 "never execute"'))
    digest = environment.digest(data)
    uart.command("rotate-receipts", "error: Busy")
    assert environment.digest(data) == digest
    enable(uart, data)  # Confirming a feature never resolves an admission.
    cancelled = status(uart.command(f'cancel-admission {accepted["id"]}'))
    assert cancelled["state"] == "cancelled"
    child = pid(uart, "run lost-operation owner-target owner-other")
    exited(uart, child, 1, 0)
    uart.command(f"reap {child}", "code=0")
    shared = snapshot(data)
    assert len(shared["records"]) == 2 and {r["subject"] for r in shared["records"]} == {1, 2}
    first = rotate(uart, data)
    assert first["records"] == 2
    uart.command(f'admission {accepted["id"]}', "error: OutcomeUnknown")
    uart.command("cat owner-target", "reply deliberately unobserved")
    second = rotate(uart, data)
    assert second["records"] == 0
    assert counters(uart) == baseline
    return {"confirmation": confirmed, "candidate_busy": busy, "unresolved_busy": True,
            "rotations": [first, second]}


def stalls(uart, data):
    baseline = counters(uart)
    uart.command("write stalled-target original")
    uart.command("write stalled-other private")
    root = pid(uart, "session stalled-target stalled-other")
    helper = pid(uart, f"helper {root} stalled-target stalled-other")
    actor(uart, root, "read")
    actor(uart, root, "stage")
    uart.command("stall files 600", "stall diagnostic armed")
    durations = []

    def control(command, expected=None):
        started = time.monotonic()
        answer = uart.command(command, expected)
        durations.append(time.monotonic() - started)
        assert durations[-1] < 2, (command, durations[-1])
        return answer

    control(f"act {root} commit", "actor state=pending")
    control(f"revoke {helper}", "access=requested")
    control("maintain-v7", "error: service busy or full")
    control("rotate-receipts", "error: service busy or full")
    control("mem", "pending_io=0")
    control("echo owner-still-responsive", "owner-still-responsive")
    control(f"permissions {helper}", "rights=0")
    deadline = time.monotonic() + 10
    while "access=unconfirmed" not in control(f"revocation {root}"):
        assert time.monotonic() < deadline
        time.sleep(.05)
    settlement = fence(uart, root, "discarded_staging=1 effects=settled", request=False)
    actor_result(uart, root, 3)  # Lost COMMIT exchange is Uncertain.
    actor(uart, helper, "read", 23)
    uart.command("cat stalled-target", "original")
    cleanup(uart, root, helper)
    assert counters(uart) == baseline

    root = pid(uart, "session stalled-target stalled-other")
    actor(uart, root, "read")
    actor(uart, root, "stage")
    uart.command("stall files 0", "stall diagnostic armed")
    control(f"act {root} commit", "actor state=pending")
    control("mem", "pending_io=0")
    control("echo recover-without-ai", "recover-without-ai")
    uart.command("restart files", "utility sessions revoked")
    uart.command("cat stalled-target", "original")
    uart.command(f"act {root} read", "error: service denied")
    assert counters(uart) == baseline
    fresh = pid(uart, "session stalled-target stalled-other")
    actor(uart, fresh, "read")
    actor(uart, fresh, "stage")
    actor(uart, fresh, "commit")
    cleanup(uart, fresh)
    uart.command("cat stalled-target", "session client edit")
    assert snapshot(data)["contents"]["/workspaces/stalled-target"] == b"session client edit"
    assert counters(uart) == baseline
    return {"finite_stall": True, "settlement": settlement.strip(), "indefinite_restart": True,
            "fresh_binding_committed": True, "max_control_seconds": round(max(durations), 3)}


def verify(image, volume_tool, output=None):
    image, volume_tool = Path(image).resolve(), Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-owner")
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").unlink(missing_ok=True)
    identity = json.loads((image.parent / "image.json").read_text())
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="rustic-v7-owner-") as temporary:
        temporary = Path(temporary)
        data = temporary / "owner.raw"
        lineage = uuid.uuid4().hex
        volume_json(volume_tool, "provision7", data, lineage)

        def first(uart):
            return {"maintenance": maintenance(uart, data), "stalls": stalls(uart, data)}

        one = boot_terminal(image, data, output, 1, temporary, first)
        after = snapshot(data)
        digest = environment.digest(data)

        def second(uart):
            uart.command("cat owner-target", "reply deliberately unobserved")
            uart.command("cat stalled-target", "session client edit")
            return enable(uart, data)

        two = boot_terminal(image, data, output, 2, temporary, second)
        assert environment.digest(data) == digest and snapshot(data) == after
        assert volume_json(volume_tool, "report7", data)["epoch"] == after["epoch"]
    result = {"outcome": "success", "returncode": 33, "timed_out": False,
              "build_id": identity["build_id"], "image_sha256": identity["image_sha256"],
              "elapsed_seconds": round(time.monotonic() - started, 3),
              "terminal_v7_owner": {"verified": True, "boots": 2, "lineage": lineage,
                                     "first": one, "second": two, "read_only_reboot_unchanged": True}}
    (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
    print("V7 owner commands: feature confirmation without writes, shared retry epoch, "
          "Busy guards, stopped service takeover/restart and fresh grants verified.", flush=True)
    return result
