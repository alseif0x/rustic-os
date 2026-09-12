# SPDX-License-Identifier: Apache-2.0
"""Public live control across real IPC while an owned VirtIO completion is held."""
import re
import time
from .cases import pid, counters
from .authority_cases import actor_result, cleanup
from .admission_cases import status, check
from .operation_cases import references, operation
from .recovery_cases import stat
from .oracle import snapshot


def activity(text):
    lines = text.replace("\r\n", "\n").splitlines()
    headers = [line for line in lines if line.startswith("admission-activity-v1 ")]
    if len(headers) != 1 or any(line.startswith("error:") for line in lines):
        raise AssertionError("missing or ambiguous live activity")
    match = re.fullmatch(r"admission-activity-v1 id=(ad_[0-9a-f]{32}_[0-9a-f]{16}) service_instance=(si_[0-9a-f]{32}_[0-9a-f]{16}) phase=(running|stopping|settling) cancel_requested=([01]) io_pending=([01])", headers[0])
    if not match:
        raise AssertionError("invalid live activity")
    return dict(id=match[1], instance=match[2], phase=match[3], requested=int(match[4]), pending=int(match[5]))


def held(uart):
    deadline = time.monotonic() + 8
    while "held=1" not in uart.command("io-status"):
        if time.monotonic() >= deadline:
            raise AssertionError("execution never reached the held native command")
        time.sleep(.02)


def act(uart, child, action, admission, expected=0):
    uart.command(f"act-admission {child} {action} {admission}", "actor state=pending")
    return actor_result(uart, child, expected)


def verify(session, owned_disk, temporary, image, mount):
    cases = []
    fault_base = None
    for name, skip, rights, scope, denial in (
        ("early", 0, 8, "hello", 0), ("header", 15, 8, "hello", 0),
        ("flush", 16, 8, "hello", 0), ("inspect_only", 0, 4, "hello", 17),
        ("foreign_scope", 0, 8, "other", 27),
    ):
        with owned_disk(temporary / ("activity-" + name + ".raw"), True, evidence_name="activity-" + name) as data:
            with session(image, data, "activity-" + name) as uart:
                uart.command("write hello before"); uart.command("write other untouched")
                node = stat(uart, "hello")
                ws, resource = references(uart, ".", "hello")
                uart.command("enable-operations", "persistent format v3")
                uart.command("enable-admissions", "persistent format v4")
                admitted = status(uart.command(f'admit-ref {ws} {resource} v_{node["version"]:016x} e_0000000000000001 k_000000000000004d "after"'))
                if fault_base is None:
                    fault_base = (snapshot(data)[0], admitted, node)
                baseline = counters(uart)
                executor = pid(uart, "admission-session hello other 7")
                controller = pid(uart, f"admission-session {scope} hello {rights}")
                # Neither inspect-only nor cancel-only may execute a retained write.
                act(uart, controller, "execute", admitted["id"], 17)
                if rights == 8:
                    act(uart, controller, "activity", admitted["id"], 17)
                uart.command("hold-io " + str(skip) + " 400", "diagnostic armed")
                uart.command(f"act-admission {executor} execute {admitted['id']}", "actor state=pending")
                held(uart)
                before = activity(uart.command(f"admission-activity {admitted['id']}"))
                if before["id"] != admitted["id"] or before["instance"] != admitted["instance"] or not before["pending"] or before["requested"]:
                    raise AssertionError("live observation lost identity or pending state")
                result = act(uart, controller, "request-cancel", admitted["id"], denial)
                if not denial and (result["other"] != 1 or result["control_denied"] != 1):
                    raise AssertionError("stop was not observed while real I/O was pending")
                after = activity(uart.command(f"admission-activity {admitted['id']}"))
                if after["id"] != admitted["id"] or after["instance"] != admitted["instance"]:
                    raise AssertionError("live control changed the retained operation identity")
                if after["requested"] != int(not denial):
                    raise AssertionError("unauthorized stop changed execution state")
                expected_phase = "settling" if skip >= 15 else "stopping" if not denial else "running"
                if after["phase"] != expected_phase or not after["pending"]:
                    raise AssertionError("cancellation ACK confused prevention and settlement")
                uart.command("io-status", "held=1")
                uart.command("mem", "pending_io=1")
                uart.command("echo owner-progress", "owner-progress")
                finished = actor_result(uart, executor)
                committed = bool(denial or skip >= 15)
                if finished["value"] != (3 if committed else 2):
                    raise AssertionError("execution result differs from the selected effect boundary")
                final = status(uart.command(f"admission {admitted['id']}"))
                check(data, final, b"after", node["id"])
                if final["state"] != ("committed" if committed else "cancelled"):
                    raise AssertionError("durable outcome differs from live control")
                completion = (operation(uart.command(f"operation op_{final['lineage']}_{final['terminal']:016x}"))
                              if committed else None)
                uart.command("cat hello", "after" if committed else "before")
                uart.command(f"admission-activity {admitted['id']}", "Unavailable")
                cleanup(uart, executor, controller)
                if counters(uart) != baseline:
                    raise AssertionError("public control leaked process/channel/I/O resources")
            with session(mount, data, "activity-" + name + "-reboot") as uart:
                if status(uart.command(f"admission {admitted['id']}")) != final:
                    raise AssertionError("reboot changed the settled result")
                uart.command("cat hello", "after" if committed else "before")
                uart.command("cat other", "untouched")
            _, observed = snapshot(data)
            if observed["nodes"][node["id"]]["content"] != (b"after" if committed else b"before"):
                raise AssertionError("independent disk contradicts the reported file effect")
            cases.append(dict(case="public_activity_" + name, verified=True, skip=skip, rights=rights,
                              denied=denial, committed=committed, status_during_io=True, reboot_verified=True,
                              observations=[before, after], durable=final, completion=completion,
                              sha256=observed["selected_sha256"]))
    base, admitted, node = fault_base
    with owned_disk(temporary / "activity-fault.raw", True, evidence_name="activity-fault") as data:
        with data.open("r+b") as stream:
            stream.write(base)
        with session(mount, data, "activity-fault", 0) as uart:
            executor = pid(uart, "admission-session hello other 7")
            controller = pid(uart, "admission-session hello other 8")
            uart.command("hold-io 0 400", "diagnostic armed")
            uart.command(f"act-admission {executor} execute {admitted['id']}", "actor state=pending")
            held(uart)
            result = act(uart, controller, "request-cancel", admitted["id"])
            if result["other"] != 1 or result["control_denied"] != 1:
                raise AssertionError("missing accepted stop during faulted pending I/O")
            actor_result(uart, executor, 3)  # Uncertain; never fabricated Cancelled.
            uart.command("restart files", "utility sessions revoked")
            if status(uart.command(f"admission {admitted['id']}")) != admitted:
                raise AssertionError("failed drain fabricated durable cancellation")
            uart.command("cat hello", "before")
        with session(mount, data, "activity-fault-reboot") as uart:
            if status(uart.command(f"admission {admitted['id']}")) != admitted:
                raise AssertionError("reboot replayed an uncertain stopped request")
            uart.command("cat hello", "before"); uart.command("cat other", "untouched")
        observed = check(data, admitted, b"after", node["id"])
        if observed["nodes"][node["id"]]["content"] != b"before":
            raise AssertionError("failed drain changed the file")
        cases.append(dict(case="public_activity_failed_drain", verified=True, uncertain=True,
                          stopped=True, durable=admitted, reboot_verified=True,
                          sha256=observed["selected_sha256"]))
    return cases
