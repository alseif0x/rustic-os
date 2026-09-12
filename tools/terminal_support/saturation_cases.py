# SPDX-License-Identifier: Apache-2.0
"""Saturated staging and client queues must not starve owner, status or stop traffic."""
from .cases import pid, counters
from .authority_cases import actor, actor_result, cleanup
from .admission_cases import status, check
from .activity_cases import activity, held
from .operation_cases import references
from .recovery_cases import stat
from .oracle import snapshot


def verify(session, owned_disk, temporary, image, mount):
    with owned_disk(temporary / "activity-saturated.raw", True, evidence_name="activity-saturated") as data:
        with session(image, data, "activity-saturated") as uart:
            uart.command("write hello before"); uart.command("write other untouched")
            node = stat(uart, "hello")
            ws, resource = references(uart, ".", "hello")
            uart.command("enable-operations", "persistent format v3")
            uart.command("enable-admissions", "persistent format v4")
            admitted = status(uart.command(f'admit-ref {ws} {resource} v_{node["version"]:016x} e_0000000000000001 k_000000000000004e "after"'))
            baseline = counters(uart)
            executor = pid(uart, "admission-session hello other 7")
            saturating = pid(uart, "admission-session hello other 3")
            # Both staging slots are retained by other clients before any execution.
            for child in (executor, saturating):
                actor(uart, child, "read")
                actor(uart, child, "stage")
            uart.command("write hello refused-while-staging-is-full", "Busy")
            uart.command("cat hello", "before")
            filled = actor(uart, saturating, "flood")
            if filled["value"] < 4 or filled["other"] == 0:
                raise AssertionError("queue pressure fixture did not saturate the client")
            uart.command("hold-io 0 400", "diagnostic armed")
            uart.command(f"act-admission {executor} execute {admitted['id']}", "actor state=pending")
            held(uart)
            # Owner console, live status and a public stop all proceed while staging
            # is full, one client never drains its replies and real I/O is pending.
            running = activity(uart.command(f"admission-activity {admitted['id']}"))
            if running["id"] != admitted["id"] or running["phase"] != "running" or not running["pending"]:
                raise AssertionError("saturation hid the live execution observation")
            uart.command("echo owner-progress", "owner-progress")
            uart.command("io-status", "held=1")
            uart.command("mem", "pending_io=1")
            uart.command("write hello refused-during-execution", "Busy")
            requested = activity(uart.command(f"request-cancel {admitted['id']}"))
            if not requested["requested"] or requested["phase"] != "stopping":
                raise AssertionError("stop was not accepted under saturation")
            stopping = activity(uart.command(f"admission-activity {admitted['id']}"))
            if stopping["phase"] != "stopping" or not stopping["requested"] or not stopping["pending"]:
                raise AssertionError("accepted stop was lost while replies were queued")
            if actor_result(uart, executor)["value"] != 2:
                raise AssertionError("prevented execution did not report a cancelled result")
            final = status(uart.command(f"admission {admitted['id']}"))
            if final["state"] != "cancelled":
                raise AssertionError("durable record contradicts the accepted stop")
            check(data, final, b"after", node["id"])
            uart.command("cat hello", "before")
            uart.command(f"admission-activity {admitted['id']}", "Unavailable")
            # A client that never drains only blocks itself; its queued replies survive.
            drained = actor(uart, saturating, "drain")
            if drained["value"] < 1:
                raise AssertionError("saturated client lost its queued replies")
            # Staging retained across the saturated execution remains authorized.
            actor(uart, executor, "commit")
            uart.command("cat hello", "session client edit")
            if status(uart.command(f"admission {admitted['id']}")) != final:
                raise AssertionError("a later commit changed the settled admission")
            cleanup(uart, executor, saturating)
            uart.command("write hello after-saturation")
            if counters(uart) != baseline:
                raise AssertionError("saturated execution leaked process/channel/I/O resources")
        with session(mount, data, "activity-saturated-reboot") as uart:
            if status(uart.command(f"admission {admitted['id']}")) != final:
                raise AssertionError("reboot changed the settled result")
            uart.command("cat hello", "after-saturation")
            uart.command("cat other", "untouched")
        _, observed = snapshot(data)
        if observed["nodes"][node["id"]]["content"] != b"after-saturation":
            raise AssertionError("independent disk contradicts the reported file effect")
    return [dict(case="public_activity_saturated", verified=True, staging_full=True,
                 undrained_client=True, owner_progress=True, stopped=True,
                 committed=False, reboot_verified=True, sha256=observed["selected_sha256"])]
