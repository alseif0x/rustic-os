# SPDX-License-Identifier: Apache-2.0
"""Actual scheduled IPC: a returning initiator, bounded pending work and recovery."""
import time
from .observations import observe, paired
from .activity_cases import activity, held, act
from .admission_cases import status, check
from .authority_cases import actor_result, cleanup, fence
from .cases import pid, counters
from .operation_cases import references, operation
from .oracle import snapshot
from .recovery_cases import stat


def prepare(uart, two=True):
    uart.command("write hello before"); uart.command("write other untouched")
    node = stat(uart, "hello")
    ws, resource = references(uart, ".", "hello")
    uart.command("enable-operations", "persistent format v3")
    uart.command("enable-admissions", "persistent format v4")
    def admit(key, value, expected=None):
        return uart.command(f'admit-ref {ws} {resource} v_{node["version"]:016x} e_0000000000000001 k_{key:016x} "{value}"', expected)
    admissions = [status(admit(80, "first"))]
    if two:
        admissions.append(status(admit(81, "second")))
        admit(82, "must-not-fit", "error: Full")
    return node, admissions


def settled(uart, admission):
    deadline = time.monotonic() + 20
    while True:
        text = uart.command(f"admission {admission['id']}", "")
        if "error: Closed" in text:
            uart.command("ps", "")
            uart.command("mem", "")
            raise AssertionError("file service closed during scheduled settlement; process evidence retained")
        if "error: Busy" not in text:
            result = status(text)
            if result["state"] != "admitted":
                return result
        if time.monotonic() >= deadline:
            raise AssertionError("scheduled operation never settled")
        time.sleep(.02)


def receipt(uart, final):
    return operation(uart.command(f"operation op_{final['lineage']}_{final['terminal']:016x}"))


def observation(uart, command, phase, pending, requested=0):
    value = activity(uart.command(command))
    if (value["phase"], value["pending"], value["requested"]) != (phase, pending, requested):
        raise AssertionError("scheduling observation contradicts the live execution boundary")
    return value


def queue_case(session, owned_disk, temporary, image, mount):
    with owned_disk(temporary / "scheduled-queue.raw", True, evidence_name="scheduled-queue") as data:
        with session(image, data, "scheduled-queue") as uart:
            node, (first, second) = prepare(uart)
            baseline = counters(uart)
            executor = pid(uart, "admission-session hello other 7")
            canceller = pid(uart, "admission-session hello other 8")
            before_observation = snapshot(data)[0]
            coherent = [observe(uart, a, "retained", "admitted") for a in (first, second)]
            clients = [dict(index=0, result=paired(uart, executor, coherent[0]))]
            if snapshot(data)[0] != before_observation:
                raise AssertionError("read-only observation changed storage")
            uart.command("hold-io 0 400", "diagnostic armed")
            ack = observation(uart, f"schedule-admission {first['id']}", "queued", 0)
            held(uart)
            running = observation(uart, f"admission-activity {first['id']}", "running", 1)
            coherent.append(observe(uart, first, "active", "running", 1))
            clients.append(dict(index=2, result=paired(uart, executor, coherent[-1])))
            coherent.append(observe(uart, second, "retained", "admitted"))
            denied_observation = act(uart, canceller, "observe", first["id"], 17)
            peer_ack = act(uart, executor, "schedule", second["id"])
            if (peer_ack["value"], peer_ack["other"], peer_ack["control_denied"]) != (4, 0, 0):
                raise AssertionError("deterministic client did not acknowledge queued work")
            duplicate = act(uart, executor, "schedule", second["id"])
            if duplicate != peer_ack:
                raise AssertionError("duplicate scheduling changed the queue acknowledgement")
            queued = observation(uart, f"admission-activity {second['id']}", "queued", 0)
            coherent.append(observe(uart, second, "active", "queued"))
            clients.append(dict(index=4, result=paired(uart, executor, coherent[-1])))
            stopped = act(uart, canceller, "request-cancel", second["id"])
            if (stopped["value"], stopped["other"], stopped["control_denied"]) != (4, 1, 0):
                raise AssertionError("cancel-only authority did not stop queued work")
            cancelled = observation(uart, f"admission-activity {second['id']}", "queued", 0, 1)
            coherent.append(observe(uart, second, "active", "queued", requested=1))
            clients.append(dict(index=5, result=paired(uart, executor, coherent[-1])))
            uart.command("echo scheduling-owner-progress", "scheduling-owner-progress")
            uart.command("io-status", "held=1")
            finals = [settled(uart, first), settled(uart, second)]
            if [s["state"] for s in finals] != ["committed", "cancelled"]:
                raise AssertionError("FIFO execution or queued prevention failed")
            completion = receipt(uart, finals[0])
            detailed = [observe(uart, final, 'retained', final['state'], profile=2,
                                prevention='none' if final['state']=='committed' else 'unknown') for final in finals]
            for final in finals:
                coherent.append(observe(uart, final, "retained", final["state"]))
                clients.append(dict(index=len(coherent)-1, result=paired(uart, executor, coherent[-1])))
            check(data, finals[0], b"first", node["id"])
            check(data, finals[1], b"second", node["id"])
            uart.command("cat hello", "first")
            uart.command(f"schedule-admission {first['id']}", "error: Unavailable")
            cleanup(uart, executor, canceller)
            if counters(uart) != baseline:
                raise AssertionError("scheduled queue leaked client or I/O resources")
        before_reboot = snapshot(data)[0]
        with session(mount, data, "scheduled-queue-reboot") as uart:
            detailed.extend(observe(uart, final, 'retained', final['state'], profile=2,
                                    prevention='none' if final['state']=='committed' else 'unknown') for final in finals)
            coherent.extend(observe(uart, final, "retained", final["state"]) for final in finals)
            for final in finals:
                if status(uart.command(f"admission {final['id']}")) != final:
                    raise AssertionError("reboot changed a scheduled result")
            uart.command("cat hello", "first"); uart.command("cat other", "untouched")
        after_reboot, observed = snapshot(data)
        if after_reboot != before_reboot:
            raise AssertionError("reboot observation rewrote or replayed retained work")
        if observed["nodes"][node["id"]]["content"] != b"first":
            raise AssertionError("queued effect differs from independent disk bytes")
    return dict(case="scheduled_queue", verified=True, reboot_verified=True,
                detailed_observations=detailed,
                coherent_observations=coherent, observation_clients=clients,
                observation_denied=denied_observation, observation_read_only=True,
                observations=[ack, running, queued, cancelled], durable=finals, completion=completion,
                retained_full=True, duplicate_same=True, cancel_only=True, owner_progress=True,
                peer_ack=peer_ack, sha256=observed["selected_sha256"])


def lost_case(session, owned_disk, temporary, image, mount):
    with owned_disk(temporary / "scheduled-lost.raw", True, evidence_name="scheduled-lost") as data:
        with session(image, data, "scheduled-lost") as uart:
            node, (admitted,) = prepare(uart, False)
            baseline = counters(uart)
            executor = pid(uart, "admission-session hello other 7")
            uart.command("hold-io 0 400", "diagnostic armed")
            uart.command(f"act-admission {executor} lost-schedule {admitted['id']}", "actor state=pending")
            actor_result(uart, executor)
            held(uart)
            running = observation(uart, f"admission-activity {admitted['id']}", "running", 1)
            stopped = observation(uart, f"request-cancel {admitted['id']}", "stopping", 1, 1)
            final = settled(uart, admitted)
            if final["state"] != "cancelled":
                raise AssertionError("lost scheduling reply blocked durable prevention")
            check(data, final, b"first", node["id"])
            act(uart, executor, "activity", admitted["id"], 1)
            cleanup(uart, executor)
            if counters(uart) != baseline:
                raise AssertionError("lost scheduling acknowledgement leaked resources")
        with session(mount, data, "scheduled-lost-reboot") as uart:
            if status(uart.command(f"admission {admitted['id']}")) != final:
                raise AssertionError("lost reply changed the retained record on reboot")
            uart.command("cat hello", "before"); uart.command("cat other", "untouched")
        _, observed = snapshot(data)
        if observed["nodes"][node["id"]]["content"] != b"before":
            raise AssertionError("lost reply caused an unexpected file effect")
    return dict(case="scheduled_lost_reply", verified=True, reboot_verified=True,
                discarded_reply=True, stale_reply_rejected=True,
                observations=[running, stopped], durable=[final], sha256=observed["selected_sha256"])


def restart_case(session, owned_disk, temporary, image, mount):
    with owned_disk(temporary / "scheduled-restart.raw", True, evidence_name="scheduled-restart") as data:
        with session(image, data, "scheduled-restart", abrupt=True) as uart:
            node, (first, second) = prepare(uart)
            uart.command("hold-io 0 400", "diagnostic armed")
            ack = observation(uart, f"schedule-admission {first['id']}", "queued", 0)
            held(uart)
            running = observation(uart, f"admission-activity {first['id']}", "running", 1)
            pending = observation(uart, f"schedule-admission {second['id']}", "queued", 0)
            uart.command("io-status", "held=1")
        with session(mount, data, "scheduled-restart-reboot") as uart:
            recovered = [status(uart.command(f"admission {a['id']}")) for a in (first, second)]
            coherent = [observe(uart, a, "retained", "admitted") for a in recovered]
            if recovered != [first, second]:
                raise AssertionError("restart replayed a scheduled request")
            uart.command("cat hello", "before")
            uart.command(f"admission-activity {second['id']}", "error: Unavailable")
            fresh = observation(uart, f"schedule-admission {second['id']}", "queued", 0)
            final = settled(uart, second)
            if final["state"] != "committed" or status(uart.command(f"admission {first['id']}")) != first:
                raise AssertionError("fresh scheduling resumed unrelated retained work")
            completion = receipt(uart, final)
            coherent.append(observe(uart, final, "retained", "committed"))
            check(data, final, b"second", node["id"])
            uart.command("cat hello", "second"); uart.command("cat other", "untouched")
        _, observed = snapshot(data)
        if observed["nodes"][node["id"]]["content"] != b"second":
            raise AssertionError("explicit resumption differs from independent disk bytes")
    return dict(case="scheduled_restart", verified=True, reboot_verified=True,
                coherent_observations=coherent,
                no_replay=True, fresh_explicit=True, recovered=recovered,
                observations=[ack, running, pending, fresh], durable=[first, final],
                completion=completion, sha256=observed["selected_sha256"])


def revoked_case(session, owned_disk, temporary, image, mount):
    with owned_disk(temporary / "scheduled-revoked.raw", True, evidence_name="scheduled-revoked") as data:
        with session(image, data, "scheduled-revoked") as uart:
            node, (first, second) = prepare(uart)
            baseline = counters(uart)
            executor = pid(uart, "admission-session hello other 7")
            uart.command("hold-io 0 400", "diagnostic armed")
            ack = observation(uart, f"schedule-admission {first['id']}", "queued", 0)
            held(uart)
            running = observation(uart, f"admission-activity {first['id']}", "running", 1)
            peer_ack = act(uart, executor, "schedule", second["id"])
            if peer_ack["value"] != 4:
                raise AssertionError("revocation fixture never queued its request")
            queued = observation(uart, f"admission-activity {second['id']}", "queued", 0)
            fence(uart, executor, "access=fenced members=1 discarded_staging=0 effects=settled")
            observation_revoked = act(uart, executor, "observe", second["id"], 18)
            finals = [settled(uart, first), settled(uart, second)]
            if [s["state"] for s in finals] != ["committed", "cancelled"]:
                raise AssertionError("revoked queue authority executed or stopped unrelated work")
            completion = receipt(uart, finals[0])
            check(data, finals[0], b"first", node["id"])
            check(data, finals[1], b"second", node["id"])
            cleanup(uart, executor)
            if counters(uart) != baseline:
                raise AssertionError("revoked scheduled work leaked resources")
        with session(mount, data, "scheduled-revoked-reboot") as uart:
            for final in finals:
                if status(uart.command(f"admission {final['id']}")) != final:
                    raise AssertionError("revocation outcome changed after reboot")
            uart.command("cat hello", "first"); uart.command("cat other", "untouched")
        _, observed = snapshot(data)
        if observed["nodes"][node["id"]]["content"] != b"first":
            raise AssertionError("revoked queued work changed the independent file oracle")
    return dict(case="scheduled_revoked", verified=True, reboot_verified=True,
                observation_revoked=observation_revoked,
                revoked_before_execution=True, peer_ack=peer_ack,
                observations=[ack, running, queued], durable=finals,
                completion=completion, sha256=observed["selected_sha256"])


def verify(session, owned_disk, temporary, image, mount):
    return [case(session, owned_disk, temporary, image, mount)
            for case in (queue_case, lost_case, restart_case, revoked_case)]
