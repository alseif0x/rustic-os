# SPDX-License-Identifier: Apache-2.0
"""A live stop whose acknowledgement is never read: the service still settles it."""
from .cases import pid, counters
from .authority_cases import actor_result, cleanup
from .admission_cases import status, check
from .activity_cases import activity, held
from .operation_cases import references
from .recovery_cases import stat
from .oracle import snapshot


def verify(session, owned_disk, temporary, image, mount):
    with owned_disk(temporary / "activity-lost-stop.raw", True, evidence_name="activity-lost-stop") as data:
        with session(image, data, "activity-lost-stop") as uart:
            uart.command("write hello before"); uart.command("write other untouched")
            node = stat(uart, "hello")
            ws, resource = references(uart, ".", "hello")
            uart.command("enable-operations", "persistent format v3")
            uart.command("enable-admissions", "persistent format v4")
            admitted = status(uart.command(f'admit-ref {ws} {resource} v_{node["version"]:016x} e_0000000000000001 k_000000000000004f "after"'))
            baseline = counters(uart)
            executor = pid(uart, "admission-session hello other 7")
            controller = pid(uart, "admission-session hello other 8")
            uart.command("hold-io 0 400", "diagnostic armed")
            uart.command(f"act-admission {executor} execute {admitted['id']}", "actor state=pending")
            held(uart)
            running = activity(uart.command(f"admission-activity {admitted['id']}"))
            if running["phase"] != "running" or running["requested"] or not running["pending"]:
                raise AssertionError("execution was not observed before the discarded stop")
            # The stop is submitted and its acknowledgement deliberately dropped.
            uart.command(f"act-admission {controller} lost-stop {admitted['id']}", "actor state=pending")
            actor_result(uart, controller)
            stopping = activity(uart.command(f"admission-activity {admitted['id']}"))
            if stopping["phase"] != "stopping" or not stopping["requested"] or not stopping["pending"]:
                raise AssertionError("an unread acknowledgement lost the accepted stop")
            if actor_result(uart, executor)["value"] != 2:
                raise AssertionError("the accepted stop did not prevent the publication")
            final = status(uart.command(f"admission {admitted['id']}"))
            if final["state"] != "cancelled":
                raise AssertionError("durable prevention was not recorded")
            check(data, final, b"after", node["id"])
            uart.command("cat hello", "before")
            uart.command(f"admission-activity {admitted['id']}", "Unavailable")
            # The undelivered reply poisons only that client's own binding; it is
            # never decoded as a later result and nothing is replayed for it.
            uart.command(f"act-admission {controller} activity {admitted['id']}", "actor state=pending")
            actor_result(uart, controller, 1)
            if status(uart.command(f"admission {admitted['id']}")) != final:
                raise AssertionError("a stale live reply changed the settled record")
            cleanup(uart, executor, controller)
            if counters(uart) != baseline:
                raise AssertionError("a discarded live reply leaked process/channel/I/O resources")
        with session(mount, data, "activity-lost-stop-reboot") as uart:
            if status(uart.command(f"admission {admitted['id']}")) != final:
                raise AssertionError("reboot changed the settled result")
            uart.command("cat hello", "before")
            uart.command("cat other", "untouched")
        _, observed = snapshot(data)
        if observed["nodes"][node["id"]]["content"] != b"before":
            raise AssertionError("independent disk contradicts the prevented effect")
    return [dict(case="public_activity_lost_stop", verified=True, discarded_reply=True,
                 stopped=True, stale_reply_rejected=True, committed=False, reboot_verified=True,
                 observations=[running, stopping], durable=final,
                 sha256=observed["selected_sha256"])]
