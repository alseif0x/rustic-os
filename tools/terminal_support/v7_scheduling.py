# SPDX-License-Identifier: Apache-2.0
"""Native V7 acceptance for bounded scheduling, live cancellation and restart.

Two fresh ``seed7 --scratch`` volumes exercise the shell's public IPC. The
first holds an active execution while a second admission queues, checks the
two-ticket bound, stops the active and queued tickets, then explicitly
reschedules a third admission and verifies FIFO terminal sequence and bytes
across a clean reboot. The second stops QEMU through the owning machine context
while an execution is held and another is queued; reboot must leave both
durable records admitted until one is explicitly scheduled again. This models
loss of the guest VM process at a held I/O boundary, not physical power loss.
"""
import base64
import hashlib
import json
from pathlib import Path
import tempfile
import time
import uuid

import environment
from . import oracle7
from .activity_cases import activity, held
from .lifecycle_observations import cancel as lifecycle_cancel
from .negotiation_cases import profiles
from .observations import observe
from .read_cases import read as read_range
from .v7_admission import check_record, decode_status
from .v7_read import volume_json
from .v7_write import LOOKUP_TIMING, boot_terminal, check_receipt, command, decode, hex16, pattern


ROOT = environment.ROOT
RETAINED = 8
SEEDED_RECORDS = 2
SIZE = 1024
QUEUE_KEYS = (0x510, 0x511, 0x512)
QUEUE_SEEDS = (11, 12, 13)
CRASH_KEYS = (0x520, 0x521)
CRASH_SEEDS = (21, 22)

DESCRIPTOR_DIGESTS = {
    "operations.get": "251cf41a611e9c6dc4bc8c13f231c6fc753f02d3ca80776bbbb42f9dcf9f74d0",
    "operations.cancel": "7d5465b4f1d30399dff079f33b9db35626f3b53a2ff718145385609a98156f45",
}


def _snapshot(data):
    return oracle7.snapshot(Path(data).read_bytes())


def _file(snapshot, identity):
    return next(item for item in snapshot["files"] if item["id"] == identity)


def _file_bytes(snapshot, identity):
    item = _file(snapshot, identity)
    return snapshot["contents"][item["path"]]


def _check_file(snapshot, identity, version, content, what):
    item = _file(snapshot, identity)
    expected = {"version": version, "size": len(content), "sha256": hashlib.sha256(content).hexdigest()}
    if any(item[field] != value for field, value in expected.items()):
        raise AssertionError(f"{what}: selected file metadata differs from the expected bytes")
    if _file_bytes(snapshot, identity) != content:
        raise AssertionError(f"{what}: selected file bytes differ from the expected bytes")
    return {"id": identity, **expected}


def _read_guest(uart, refs, version, expected, what):
    # A one-byte request also gives a bounded EOF answer for seed7's empty file.
    length = min(1024, max(1, len(expected)))
    result = read_range(uart, refs, 0, length, version=f"v_{hex16(version)}")["result"]
    observed = base64.b64decode(result["data"])
    if result["version"] != f"v_{hex16(version)}" or result["size"] != len(expected) \
            or observed != expected or result["range_sha256"] != hashlib.sha256(expected).hexdigest():
        raise AssertionError(f"{what}: guest read differs from the pinned bytes")
    return {"version": result["version"], "size": result["size"], "sha256": result["range_sha256"],
            "bytes": len(observed)}


def _status(uart, identity):
    text = uart.command(f"admission {identity['id']}", "")
    return decode_status(text)


def _status_core(value):
    return {key: value[key] for key in ("id", "lineage", "number", "instance", "instance_sequence", "state",
                                         "terminal", "completion", "lines")}


def _expect_status(actual, expected, what):
    if _status_core(actual) != _status_core(expected):
        raise AssertionError(f"{what}: retained admission status changed")


def _settled(uart, identity, what, timeout=45):
    deadline = time.monotonic() + timeout
    while True:
        value = _status(uart, identity)
        if "error" not in value:
            if value["state"] != "admitted":
                return value
        elif value["error"] != "Busy":
            raise AssertionError(f"{what}: status query failed: {value['error']}")
        if time.monotonic() >= deadline:
            raise AssertionError(f"{what}: scheduled admission never settled")
        time.sleep(0.02)


def _activity(uart, verb, identity, phase, requested, pending):
    value = activity(uart.command(f"{verb} {identity['id']}"))
    expected = (identity["id"], identity["instance"], phase, requested, pending)
    observed = (value["id"], value["instance"], value["phase"], value["requested"], value["pending"])
    if observed != expected:
        raise AssertionError(f"{verb}: live state {observed!r} differs from {expected!r}")
    return value


def _unavailable(uart, verb, identity):
    text = uart.command(f"{verb} {identity['id']}", "error: Unavailable")
    errors = [line.strip() for line in text.replace("\r\n", "\n").splitlines() if line.startswith("error:")]
    if errors != ["error: Unavailable"]:
        raise AssertionError(f"{verb}: expected one Unavailable refusal, got {text!r}")
    return "Unavailable"


def _admit(uart, refs, version, epoch, key, seed):
    text = command(refs["workspace"], refs["resource"], version, epoch, key, seed, SIZE)
    text = text.replace("replace-pattern-v7", "admit-pattern-v7", 1)
    value = decode_status(uart.command(text, "\n"), timing=True)
    if value.get("state") != "admitted" or value["size"] != SIZE:
        raise AssertionError(f"admission was not durably retained: {value}")
    return value


def _expected(workspace, object_id, epoch, key, version, seed):
    content = pattern(seed, SIZE)
    return {"workspace": workspace, "object": object_id, "epoch": epoch, "key": key,
            "previous": version, "length": len(content), "sha256": hashlib.sha256(content).hexdigest()}


def _admit_many(uart, refs, scratch, epoch, keys, seeds):
    values = [_admit(uart, refs, scratch["version"], epoch, key, seed)
              for key, seed in zip(keys, seeds, strict=True)]
    if len({value["id"] for value in values}) != len(values) or len({value["instance"] for value in values}) != 1:
        raise AssertionError("admission identities were reused or crossed service instances")
    return values


def _check_prepared(snapshot, values, seeded, epoch, keys, seeds, scratch_bytes, what):
    if len(snapshot["records"]) != SEEDED_RECORDS + len(values):
        raise AssertionError(f"{what}: retained record count differs from the seed and admissions")
    for value, key, seed in zip(values, keys, seeds, strict=True):
        check_record(snapshot, value, _expected(seeded["workspace"]["id"], seeded["scratch"]["id"],
                                                epoch, key, seeded["scratch"]["version"], seed))
    _check_file(snapshot, seeded["scratch"]["id"], seeded["scratch"]["version"], scratch_bytes, what)


def _descriptor(uart, data):
    before = environment.digest(data)
    values = profiles(uart, True)
    after = environment.digest(data)
    if before != after:
        raise AssertionError("lifecycle DESCRIBE changed the V7 volume")
    by_method = {value["method"]: value for value in values}
    if set(by_method) != set(DESCRIPTOR_DIGESTS):
        raise AssertionError("DESCRIBE omitted a reviewed lifecycle method")
    for method, digest in DESCRIPTOR_DIGESTS.items():
        value = by_method[method]
        if (value["version"], value["profile"], value["availability"], value["retained"],
                value["tickets"], value["active"], value["sha256"]) != (2, 1, "available", RETAINED, 2, 1, digest):
            raise AssertionError(f"{method} DESCRIBE differs from its reviewed contract and mounted limits")
    if len({(value["responder"], value["context"]) for value in values}) != 1:
        raise AssertionError("lifecycle DESCRIBE methods disagree on their live responder context")
    return values


def _operation_receipt(uart, final, lineage, refs, epoch, key, version, seed):
    if final.get("state") != "committed" or not final.get("completion"):
        raise AssertionError("scheduled admission did not produce a completion")
    value = decode(uart.command(f"operation-v7 {final['completion']}", "\n"), LOOKUP_TIMING)
    if "error" in value:
        raise AssertionError(f"completion lookup failed: {value['error']}")
    check_receipt(value, lineage, refs["workspace"], refs["resource"], version, epoch, key, pattern(seed, SIZE))
    return value


def _queue_case(image, volume_tool, temporary, output):
    data = temporary / "scheduling-queue.raw"
    lineage = uuid.uuid4().hex
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
    if seeded["lineage"] != lineage:
        raise AssertionError("seed7 created a volume with another lineage")
    refs = {"workspace": seeded["workspace"]["text"], "resource": seeded["scratch"]["resource"]}
    initial = _snapshot(data)
    if len(initial["records"]) != SEEDED_RECORDS or initial["lineage"] != lineage:
        raise AssertionError("fresh scheduling fixture has an unexpected retained set or lineage")
    epoch = initial["epoch"]
    scratch_bytes = _file_bytes(initial, seeded["scratch"]["id"])
    def first_boot(uart):
        values = _admit_many(uart, refs, seeded["scratch"], epoch, QUEUE_KEYS, QUEUE_SEEDS)
        prepared = _snapshot(data)
        _check_prepared(prepared, values, seeded, epoch, QUEUE_KEYS, QUEUE_SEEDS, scratch_bytes,
                        "queue admissions")
        if prepared["sequence"] != initial["sequence"] + len(values):
            raise AssertionError("each accepted admission must publish exactly one retained generation")

        descriptors = _descriptor(uart, data)
        uart.command("select-lifecycle operations.cancel",
                     "lifecycle-selected method=operations.cancel availability=available")
        uart.command("hold-io 0 400", "diagnostic armed")
        first_ack = _activity(uart, "schedule-admission", values[0], "queued", 0, 0)
        held(uart)
        active = _activity(uart, "admission-activity", values[0], "running", 0, 1)
        active_v2 = observe(uart, values[0], "active", "running", pending=1, requested=0, profile=2)

        second_ack = _activity(uart, "schedule-admission", values[1], "queued", 0, 0)
        repeated_ack = _activity(uart, "schedule-admission", values[1], "queued", 0, 0)
        if repeated_ack != second_ack:
            raise AssertionError("repeated SCHEDULE changed the queued acknowledgement")
        queued_v2 = observe(uart, values[1], "active", "queued", pending=0, requested=0, profile=2)
        unrelated_v2 = observe(uart, values[2], "retained", "admitted", profile=2, prevention="none")
        busy_text = uart.command(f"schedule-admission {values[2]['id']}", "error: Busy")
        busy_errors = [line.strip() for line in busy_text.replace("\r\n", "\n").splitlines()
                       if line.startswith("error:")]
        if busy_errors != ["error: Busy"]:
            raise AssertionError(f"third schedule did not fail exactly Busy: {busy_text!r}")

        active_stop = _activity(uart, "request-cancel", values[0], "stopping", 1, 1)
        active_stop_repeat = _activity(uart, "request-cancel", values[0], "stopping", 1, 1)
        if active_stop_repeat != active_stop:
            raise AssertionError("repeated REQUEST_CANCEL changed the active stop state")
        active_v2_stop = observe(uart, values[0], "active", "stopping", pending=1, requested=1, profile=2)
        queued_cancel = lifecycle_cancel(uart, values[1], "requested", selected=True)
        queued_cancel_repeat = lifecycle_cancel(uart, values[1], "already_requested", selected=True)
        queued_stop = _activity(uart, "admission-activity", values[1], "queued", 1, 0)
        queued_v2_stop = observe(uart, values[1], "active", "queued", pending=0, requested=1, profile=2)
        uart.command("io-status", "held=1")
        uart.command("echo scheduling-control-progress", "scheduling-control-progress")

        finals = [_settled(uart, values[0], "active cancellation"),
                  _settled(uart, values[1], "queued cancellation")]
        if [value["state"] for value in finals] != ["cancelled", "cancelled"]:
            raise AssertionError("active and queued cancellation did not both settle as cancelled")
        if not finals[0]["terminal"] < finals[1]["terminal"]:
            raise AssertionError("queued cancellation settled before the active FIFO head")
        cancelled_v2 = [observe(uart, final, "retained", "cancelled", profile=2, prevention="requested")
                        for final in finals]
        after_cancellation = _snapshot(data)
        if len(after_cancellation["records"]) != SEEDED_RECORDS + 3:
            raise AssertionError("FIFO cancellation changed the prepared retained inventory")
        for final, key, seed in zip(finals, QUEUE_KEYS[:2], QUEUE_SEEDS[:2], strict=True):
            check_record(after_cancellation, final,
                         _expected(seeded["workspace"]["id"], seeded["scratch"]["id"], epoch, key,
                                  seeded["scratch"]["version"], seed))
        third_before_reschedule = _status(uart, values[2])
        _expect_status(third_before_reschedule, values[2], "unscheduled third admission")
        check_record(after_cancellation, third_before_reschedule,
                     _expected(seeded["workspace"]["id"], seeded["scratch"]["id"], epoch, QUEUE_KEYS[2],
                              seeded["scratch"]["version"], QUEUE_SEEDS[2]))
        third_prepared_v2 = observe(uart, third_before_reschedule, "retained", "admitted", profile=2,
                                   prevention="none")
        _check_file(after_cancellation, seeded["scratch"]["id"], seeded["scratch"]["version"],
                    scratch_bytes, "requested cancellations")
        unchanged_read = _read_guest(uart, refs, seeded["scratch"]["version"], scratch_bytes,
                                     "requested cancellations")

        # The third admission had no ticket when the first two occupied both
        # slots. A fresh SCHEDULE after settlement must now execute it.
        uart.command("hold-io 0 400", "diagnostic armed")
        third_ack = _activity(uart, "schedule-admission", values[2], "queued", 0, 0)
        held(uart)
        third_active = _activity(uart, "admission-activity", values[2], "running", 0, 1)
        third_active_v2 = observe(uart, values[2], "active", "running", pending=1, requested=0, profile=2)
        uart.command("io-status", "held=1")
        uart.command("echo third-scheduling-progress", "third-scheduling-progress")
        third_final = _settled(uart, values[2], "third admission after queue release")
        if third_final["state"] != "committed":
            raise AssertionError("fresh scheduling after queue release did not commit the third admission")
        if not finals[1]["terminal"] < third_final["terminal"]:
            raise AssertionError("the third publication did not follow the two FIFO cancellation outcomes")
        third_v2 = observe(uart, third_final, "retained", "committed", profile=2, prevention="none")
        receipt = _operation_receipt(uart, third_final, lineage, refs, epoch, QUEUE_KEYS[2],
                                     seeded["scratch"]["version"], QUEUE_SEEDS[2])

        final_snapshot = _snapshot(data)
        if len(final_snapshot["records"]) != SEEDED_RECORDS + 3:
            raise AssertionError("queue acceptance or settlement changed the expected retained count")
        for final, key, seed in zip([finals[0], finals[1], third_final], QUEUE_KEYS, QUEUE_SEEDS, strict=True):
            check_record(final_snapshot, final,
                         _expected(seeded["workspace"]["id"], seeded["scratch"]["id"], epoch, key,
                                  seeded["scratch"]["version"], seed))
        written = pattern(QUEUE_SEEDS[2], SIZE)
        _check_file(final_snapshot, seeded["scratch"]["id"], third_final["terminal"], written,
                    "third scheduled commit")
        committed_read = _read_guest(uart, refs, third_final["terminal"], written, "third scheduled commit")
        for value in values:
            _unavailable(uart, "admission-activity", value)
        return {
            "accepted": values,
            "prepared_sequence": prepared["sequence"],
            "descriptor": descriptors,
            "queue": {"first": first_ack, "active": active, "second": second_ack,
                      "repeated_second": repeated_ack, "third_refusal": "Busy",
                      "observations_v2": [active_v2, queued_v2, unrelated_v2]},
            "cancellation": {"active_request_cancel": active_stop,
                             "active_request_cancel_repeat": active_stop_repeat,
                             "active_observation_v2": active_v2_stop,
                             "queued_lifecycle_cancel": queued_cancel,
                             "queued_lifecycle_cancel_repeat": queued_cancel_repeat,
                             "queued_activity": queued_stop, "queued_observation_v2": queued_v2_stop,
                             "durable_observations_v2": cancelled_v2,
            "third_prepared_observation_v2": third_prepared_v2},
            "cancelled": finals,
            "fresh_schedule": {"queued": third_ack, "running": third_active,
                               "running_observation_v2": third_active_v2},
            "committed": third_final,
            "completion": receipt,
            "oracle": {"after_cancellation_sequence": after_cancellation["sequence"],
                       "after_cancellation_file": _check_file(after_cancellation, seeded["scratch"]["id"],
                                                              seeded["scratch"]["version"], scratch_bytes,
                                                              "cancellation evidence"),
                       "after_commit_sequence": final_snapshot["sequence"],
                       "after_commit_file": _check_file(final_snapshot, seeded["scratch"]["id"],
                                                        third_final["terminal"], written, "commit evidence"),
                       "read_before_reschedule": unchanged_read, "read_after_commit": committed_read},
        }

    queue_output = output / "queue"
    queue_output.mkdir(parents=True, exist_ok=True)
    boot_temporary = temporary / "queue-boot"
    boot_temporary.mkdir()
    first = boot_terminal(image, data, queue_output, 1, boot_temporary, first_boot)
    after_first = _snapshot(data)
    first_finals = [*first["cancelled"], first["committed"]]
    if after_first["sequence"] != first["oracle"]["after_commit_sequence"]:
        raise AssertionError("clean boot-1 shutdown changed the selected V7 sequence")
    before_reboot = environment.digest(data)

    def second_boot(uart):
        observed = []
        for final in first_finals:
            current = _status(uart, final)
            _expect_status(current, final, "queue reboot")
            prevention = "none" if final["state"] == "committed" else "requested"
            observed.append(observe(uart, current, "retained", final["state"], profile=2,
                                    prevention=prevention))
            _unavailable(uart, "admission-activity", final)
        written = pattern(QUEUE_SEEDS[2], SIZE)
        read = _read_guest(uart, refs, first["committed"]["terminal"], written, "queue reboot")
        return {"statuses": observed, "read": read}

    reboot = boot_terminal(image, data, queue_output, 2, boot_temporary, second_boot)
    after_reboot = _snapshot(data)
    if environment.digest(data) != before_reboot or after_reboot["sequence"] != after_first["sequence"]:
        raise AssertionError("read-only queue reboot changed the V7 volume")
    if _file_bytes(after_reboot, seeded["scratch"]["id"]) != pattern(QUEUE_SEEDS[2], SIZE):
        raise AssertionError("queued commit bytes changed across clean reboot")
    report = volume_json(volume_tool, "report7", data)
    if report["lineage"] != lineage or report["sequence"] != after_reboot["sequence"] \
            or len(report["records"]) != SEEDED_RECORDS + 3:
        raise AssertionError("report7 and oracle7 disagree after queue reboot")
    return {"lineage": lineage, "initial": {"sequence": initial["sequence"], "records": len(initial["records"])},
            "boot_1": first, "after_boot_1": {"sequence": after_first["sequence"],
                                             "records": len(after_first["records"])},
            "boot_2": reboot, "after_boot_2": {"sequence": after_reboot["sequence"],
                                               "records": len(after_reboot["records"])}}


def _crash_case(image, volume_tool, temporary, output):
    data = temporary / "scheduling-abrupt.raw"
    lineage = uuid.uuid4().hex
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
    if seeded["lineage"] != lineage:
        raise AssertionError("seed7 created a volume with another lineage")
    refs = {"workspace": seeded["workspace"]["text"], "resource": seeded["scratch"]["resource"]}
    initial = _snapshot(data)
    if len(initial["records"]) != SEEDED_RECORDS or initial["lineage"] != lineage:
        raise AssertionError("fresh abrupt fixture has an unexpected retained set or lineage")
    epoch = initial["epoch"]
    scratch_bytes = _file_bytes(initial, seeded["scratch"]["id"])
    def held_boot(uart):
        values = _admit_many(uart, refs, seeded["scratch"], epoch, CRASH_KEYS, CRASH_SEEDS)
        prepared = _snapshot(data)
        _check_prepared(prepared, values, seeded, epoch, CRASH_KEYS, CRASH_SEEDS, scratch_bytes,
                        "abrupt admissions")
        uart.command("hold-io 0 400", "diagnostic armed")
        first_ack = _activity(uart, "schedule-admission", values[0], "queued", 0, 0)
        held(uart)
        active = _activity(uart, "admission-activity", values[0], "running", 0, 1)
        active_v2 = observe(uart, values[0], "active", "running", pending=1, requested=0, profile=2)
        second_ack = _activity(uart, "schedule-admission", values[1], "queued", 0, 0)
        queued_v2 = observe(uart, values[1], "active", "queued", pending=0, requested=0, profile=2)
        uart.command("io-status", "held=1")
        return {"accepted": values, "prepared_sequence": prepared["sequence"],
                "schedule": [first_ack, second_ack], "activity": [active],
                "observations_v2": [active_v2, queued_v2],
                "held_boundary": "first native I/O held and second ticket queued before QEMU termination"}

    crash_output = output / "abrupt"
    crash_output.mkdir(parents=True, exist_ok=True)
    boot_temporary = temporary / "abrupt-boot"
    boot_temporary.mkdir()
    abrupt_boot = boot_terminal(image, data, crash_output, 1, boot_temporary, held_boot, abrupt=True)
    if abrupt_boot["termination_boundary"].get("kind") != "machine_context_terminated_qemu" \
            or abrupt_boot["termination_boundary"].get("guest_clean_exit_requested") is not False:
        raise AssertionError("abrupt boot did not record the owned-VM termination boundary")
    values = abrupt_boot["accepted"]
    after_crash = _snapshot(data)
    _check_prepared(after_crash, values, seeded, epoch, CRASH_KEYS, CRASH_SEEDS, scratch_bytes,
                    "after abrupt VM termination")
    if after_crash["sequence"] != abrupt_boot["prepared_sequence"]:
        raise AssertionError("held execution published a terminal result before abrupt VM termination")

    before_reboot = environment.digest(data)

    def recovery_boot(uart):
        recovered = []
        observations = []
        for original in values:
            current = _status(uart, original)
            _expect_status(current, original, "abrupt reboot")
            if current["state"] != "admitted":
                raise AssertionError("volatile tickets resumed execution across terminal-v7 restart")
            observations.append(observe(uart, current, "retained", "admitted", profile=2, prevention="none"))
            _unavailable(uart, "admission-activity", original)
            recovered.append(current)
        unchanged = _read_guest(uart, refs, seeded["scratch"]["version"], scratch_bytes,
                                "before fresh post-crash scheduling")

        uart.command("hold-io 0 400", "diagnostic armed")
        fresh = _activity(uart, "schedule-admission", values[0], "queued", 0, 0)
        held(uart)
        active = _activity(uart, "admission-activity", values[0], "running", 0, 1)
        active_v2 = observe(uart, values[0], "active", "running", pending=1, requested=0, profile=2)
        uart.command("io-status", "held=1")
        uart.command("echo post-crash-scheduling-progress", "post-crash-scheduling-progress")
        final = _settled(uart, values[0], "explicit post-crash scheduling")
        if final["state"] != "committed":
            raise AssertionError("explicit rescheduling after restart did not commit")
        remaining = _status(uart, values[1])
        if _status_core(remaining) != _status_core(values[1]):
            raise AssertionError("fresh scheduling also resumed the unrelated queued admission")
        observations.append(observe(uart, final, "retained", "committed", profile=2, prevention="none"))
        observations.append(observe(uart, remaining, "retained", "admitted", profile=2, prevention="none"))
        receipt = _operation_receipt(uart, final, lineage, refs, epoch, CRASH_KEYS[0],
                                     seeded["scratch"]["version"], CRASH_SEEDS[0])
        content = pattern(CRASH_SEEDS[0], SIZE)
        committed_read = _read_guest(uart, refs, final["terminal"], content, "explicit post-crash commit")
        return {"recovered": recovered, "remaining": remaining, "observations_v2": observations,
                "pre_schedule_read": unchanged, "fresh_schedule": fresh,
                "active": active, "active_observation_v2": active_v2, "committed": final,
                "completion": receipt, "committed_read": committed_read}

    reboot = boot_terminal(image, data, crash_output, 2, boot_temporary, recovery_boot)
    after_reboot = _snapshot(data)
    if environment.digest(data) == before_reboot:
        # The explicit reschedule must publish a new terminal result.
        raise AssertionError("fresh post-crash scheduling did not change the V7 image")
    if after_reboot["sequence"] != reboot["committed"]["terminal"]:
        raise AssertionError("post-crash commit sequence differs from its retained result")
    if after_reboot["sequence"] <= after_crash["sequence"]:
        raise AssertionError("explicit post-crash scheduling did not advance the V7 sequence")
    if len(after_reboot["records"]) != SEEDED_RECORDS + 2:
        raise AssertionError("abrupt restart changed the bounded retained inventory")
    final_first = reboot["committed"]
    expected_first = _expected(seeded["workspace"]["id"], seeded["scratch"]["id"], epoch,
                               CRASH_KEYS[0], seeded["scratch"]["version"], CRASH_SEEDS[0])
    check_record(after_reboot, final_first, expected_first)
    expected_second = _expected(seeded["workspace"]["id"], seeded["scratch"]["id"], epoch,
                                CRASH_KEYS[1], seeded["scratch"]["version"], CRASH_SEEDS[1])
    check_record(after_reboot, reboot["remaining"], expected_second)
    _check_file(after_reboot, seeded["scratch"]["id"], final_first["terminal"],
                pattern(CRASH_SEEDS[0], SIZE), "post-crash explicit commit")
    report = volume_json(volume_tool, "report7", data)
    if report["lineage"] != lineage or report["sequence"] != after_reboot["sequence"] \
            or len(report["records"]) != SEEDED_RECORDS + 2:
        raise AssertionError("report7 and oracle7 disagree after explicit post-crash scheduling")
    return {"lineage": lineage, "initial": {"sequence": initial["sequence"], "records": len(initial["records"])},
            "boot_1": abrupt_boot, "after_abrupt_exit": {"sequence": after_crash["sequence"],
                                                    "records": len(after_crash["records"]),
                                                    "file": _check_file(after_crash, seeded["scratch"]["id"],
                                                                        seeded["scratch"]["version"], scratch_bytes,
                                                                        "abrupt reboot baseline")},
            "boot_2": reboot, "after_boot_2": {"sequence": after_reboot["sequence"],
                                                "records": len(after_reboot["records"]),
                                                "file": _check_file(after_reboot, seeded["scratch"]["id"],
                                                                    final_first["terminal"],
                                                                    pattern(CRASH_SEEDS[0], SIZE),
                                                                    "post-crash explicit commit")}}


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-scheduling")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    for path, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(path) != metadata["native_applications"]["file-server"][suffix]:
            raise RuntimeError(f"{path.name} differs from the artifact recorded by the boot build")
    (output / "result.json").unlink(missing_ok=True)
    started = time.monotonic()
    phases = (("queue", 1), ("queue", 2), ("abrupt", 1), ("abrupt", 2))
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-scheduling-") as temporary:
            temporary = Path(temporary)
            queue = _queue_case(image, volume_tool, temporary, output)
            abrupt = _crash_case(image, volume_tool, temporary, output)
        evidence = {"verified": True, "mode": "terminal-v7", "boots": 4,
                    "lineages": {"queue": queue["lineage"], "abrupt": abrupt["lineage"]},
                    "retained_limit": RETAINED, "execution_tickets": 2, "active_publications": 1,
                    "queue_case": queue, "abrupt_case": abrupt,
                    "abrupt_boundary": "QEMU process terminated by machine context while native I/O was held; no guest clean exit or hardware power-loss claim"}
        result = {"outcome": "success", "returncode": 33, "timed_out": False,
                  "elapsed_seconds": round(time.monotonic() - started, 3),
                  "build_id": metadata["build_id"], "image_sha256": metadata["image_sha256"],
                  "terminal_v7_scheduling": evidence}
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print("V7 scheduling acceptance: the public queue admitted one active and one queued ticket, refused a third, "
              "exposed stable v2 activity and negotiated DESCRIBE, and persisted requested cancellation in FIFO "
              "order; a freed queue explicitly committed the third admission and matched oracle7 before and after "
              "clean reboot. A separate held-I/O case terminated the QEMU process at the machine-context boundary; "
              "both admissions remained admitted after reboot until one was explicitly rescheduled. This does not "
              "claim hardware power-loss behavior.", flush=True)
        return result
    finally:
        for kind in ("serial", "qemu"):
            parts = []
            for case, phase in phases:
                path = output / case / f"{kind}-{phase}.log"
                if path.exists():
                    parts.append(path.read_bytes())
            (output / f"{kind}.log").write_bytes(b"\n".join(parts))
