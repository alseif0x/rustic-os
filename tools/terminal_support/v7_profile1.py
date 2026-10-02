# SPDX-License-Identifier: Apache-2.0
"""Native acceptance of the existing profile-1 operation and admission APIs on V7.

The fixture is a fresh, temporary `seed7 --scratch` volume used in two
disposable terminal boots. It exercises the existing unmarked SDK requests
through the existing shell commands, and checks receipts and retained bytes
with `oracle7`. One profile-2 write above profile 1's 1024-byte receipt limit
also confirms that profile-1 lookup refuses it while the existing profile-2
lookup can still retrieve it.
"""
import hashlib
import json
from pathlib import Path
import tempfile
import time
import uuid

import environment
from . import oracle7
from .admission_cases import status as decode_admission_status
from .operation_cases import check as check_operation_receipt
from .operation_cases import operation as decode_operation_receipt
from .v7_admission import _expected as expected_admission_record
from .v7_admission import check_record as check_admission_record
from .v7_admission import decode_status as decode_v7_admission_status
from .v7_read import volume_json
from .v7_write import LOOKUP_TIMING, boot_terminal, check_receipt, command as pattern_command
from .v7_write import decode as decode_profile2_receipt
from .v7_write import hex16, match_records, pattern


ROOT = environment.ROOT
SHELL_SUBJECT = 2
SEEDED_RECORDS = 2
TRACKED_KEY = 0x610
PROFILE2_KEY = 0x611
EXECUTE_KEY = 0x612
CANCEL_KEY = 0x613
TRACKED_CONTENT = b"profile 1 tracked V7"
EXECUTE_CONTENT = b"profile 1 admitted and executed"
CANCEL_CONTENT = b"profile 1 admitted then cancelled"
PROFILE2_SIZE = 1025
PROFILE2_SEED = 17


def _live(data):
    """The independent on-disk view while the guest is idle."""
    return oracle7.snapshot(data.read_bytes())


def _records(snapshot, expected, phase):
    count = len(snapshot["records"])
    if count != expected:
        raise AssertionError(f"{phase}: oracle7 finds {count} retained records, expected {expected}")
    if count > oracle7.RETAINED:
        raise AssertionError(f"{phase}: the retained record count exceeds the V7 limit")


def _file(snapshot, identity):
    found = [item for item in snapshot["files"] if item["id"] == identity]
    if len(found) != 1:
        raise AssertionError(f"oracle7 finds {len(found)} live files with identity {identity}")
    return found[0]


def _check_live_bytes(snapshot, identity, version, content, what):
    node = _file(snapshot, identity)
    digest = hashlib.sha256(content).hexdigest()
    if (node["version"], node["size"], node["sha256"]) != (version, len(content), digest):
        raise AssertionError(f"{what}: live metadata does not match the expected version and bytes")
    if snapshot["contents"].get(node["path"]) != content:
        raise AssertionError(f"{what}: oracle7 decoded different live bytes")
    return node


def _receipt_lines(text):
    """Return only the canonical old-format receipt lines after strict decoding."""
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    receipt = [line for line in lines if line.startswith(("operation-v1 ", "receipt workspace="))]
    if len(receipt) != 2 or not receipt[0].startswith("operation-v1 ") \
            or not receipt[1].startswith("receipt workspace="):
        raise AssertionError(f"missing or ambiguous profile-1 receipt lines: {text!r}")
    return receipt


def _profile1_replace_command(refs, previous, epoch, key, content):
    text = content.decode("ascii")
    if any(char in text for char in ('"', "\r", "\n")):
        raise ValueError("profile-1 acceptance content must fit one plain shell argument")
    return (f'replace-ref {refs["workspace"]} {refs["resource"]} v_{hex16(previous)} '
            f'e_{hex16(epoch)} k_{hex16(key)} "{text}"')


def _profile1_replace(uart, refs, previous, epoch, key, content):
    command = _profile1_replace_command(refs, previous, epoch, key, content)
    text = uart.command(command, "\n")
    receipt = decode_operation_receipt(text)
    check_operation_receipt(receipt, refs["workspace"], refs["resource"], previous, epoch, key, content)
    return {"command": command, "receipt": receipt, "lines": _receipt_lines(text)}


def _profile1_operation(uart, command, expected, what):
    text = uart.command(command, "\n")
    receipt = decode_operation_receipt(text)
    if receipt != expected["receipt"] or _receipt_lines(text) != expected["lines"]:
        raise AssertionError(f"{what}: profile-1 lookup differs from the exact original receipt")
    return {"receipt": receipt, "lines": _receipt_lines(text)}


def _single_error(text, name):
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    errors = [line for line in lines if line.startswith("error:")]
    if errors != [f"error: {name}"] or any(line.startswith(("operation-v1 ", "receipt ")) for line in lines):
        raise AssertionError(f"expected one profile-1 {name} refusal, got {text!r}")


def _profile2_write(uart, refs, lineage, previous, epoch, key):
    content = pattern(PROFILE2_SEED, PROFILE2_SIZE)
    text = uart.command(
        pattern_command(refs["workspace"], refs["resource"], previous, epoch, key,
                       PROFILE2_SEED, PROFILE2_SIZE),
        "\n",
    )
    receipt = decode_profile2_receipt(text)
    check_receipt(receipt, lineage, refs["workspace"], refs["resource"], previous, epoch, key, content)
    return {"receipt": receipt, "lines": receipt["lines"], "content": content}


def _profile2_lookup(uart, command, expected, what):
    text = uart.command(command, "\n")
    receipt = decode_profile2_receipt(text, LOOKUP_TIMING)
    if receipt["lines"] != expected["lines"] or receipt["size"] != PROFILE2_SIZE:
        raise AssertionError(f"{what}: profile-2 lookup did not return the exact retained receipt")
    return receipt


def _check_profile1_record(snapshot, refs_ids, operation, content, what):
    workspace_id, object_id = refs_ids
    parsed = operation["receipt"]
    version = int(parsed["version"][2:], 16)
    previous = int(parsed["previous_version"][2:], 16)
    epoch = int(parsed["retry"]["epoch"][2:], 16)
    key = int(parsed["retry"]["key"][2:], 16)
    records = [record for record in snapshot["records"]
               if record["subject"] == SHELL_SUBJECT and record["workspace"] == workspace_id
               and record["object"] == object_id and record["epoch"] == epoch and record["key"] == key]
    if len(records) != 1:
        raise AssertionError(f"{what}: oracle7 finds {len(records)} matching profile-1 records")
    record = records[0]
    expected = {
        "state": "direct_committed", "previous": previous, "committed": version,
        "terminal": version, "admission": 0, "length": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
    }
    for field, value in expected.items():
        if record[field] != value:
            raise AssertionError(f"{what}: oracle7 record {field}={record[field]!r}, expected {value!r}")
    instance = int(operation["service_instance"].rsplit("_", 1)[1], 16)
    if record["instance"] != instance:
        raise AssertionError(f"{what}: receipt and oracle7 service instances differ")
    return {key: record[key] for key in ("slot", "state", "previous", "committed", "length", "sha256")}


def _profile1_admit(uart, refs, previous, epoch, key, content):
    command = _profile1_replace_command(refs, previous, epoch, key, content).replace("replace-ref ", "admit-ref ", 1)
    text = uart.command(command, "\n")
    status = decode_v7_admission_status(text)
    if "error" in status:
        raise AssertionError(f"profile-1 admission failed: {status['error']}")
    if status["state"] != "admitted":
        raise AssertionError(f"profile-1 admission did not remain admitted: {status}")
    # This existing decoder enforces the v1 status header/completion shape.
    decode_admission_status(text)
    return {"command": command, "status": status}


def _check_admission(snapshot, status, refs_ids, epoch, key, previous, content, what):
    checked = check_admission_record(
        snapshot, status, expected_admission_record(refs_ids, epoch, key, previous, content)
    )
    if snapshot["lineage"] != status["lineage"]:
        raise AssertionError(f"{what}: admission status names another V7 lineage")
    return checked


def _same_admission_status(uart, commands, expected, data, what):
    before = environment.digest(data)
    observed = []
    for command in commands:
        text = uart.command(command, "\n")
        status = decode_v7_admission_status(text)
        if status.get("lines") != expected["status"]["lines"]:
            raise AssertionError(f"{what}: {command!r} returned different profile-1 status lines")
        observed.append(command)
    if environment.digest(data) != before:
        raise AssertionError(f"{what}: replay or lookup changed the image")
    return observed


def _status_command(uart, command):
    text = uart.command(command, "\n")
    status = decode_v7_admission_status(text)
    if "error" in status:
        raise AssertionError(f"{command!r} failed with {status['error']}")
    decode_admission_status(text)
    return status


def _first_boot(uart, data, refs, refs_ids, lineage, state):
    before = _live(data)
    _records(before, SEEDED_RECORDS, "seed")

    tracked = _profile1_replace(uart, refs, state["version"], state["epoch"], TRACKED_KEY, TRACKED_CONTENT)
    tracked_version = int(tracked["receipt"]["receipt"]["version"][2:], 16)
    tracked_live = _live(data)
    _records(tracked_live, SEEDED_RECORDS + 1, "profile-1 tracked commit")
    tracked_record = _check_profile1_record(tracked_live, refs_ids, tracked["receipt"], TRACKED_CONTENT,
                                            "profile-1 tracked commit")
    _check_live_bytes(tracked_live, refs_ids[1], tracked_version, TRACKED_CONTENT, "profile-1 tracked commit")
    if tracked_live["sequence"] != before["sequence"] + 1:
        raise AssertionError("profile-1 tracked commit did not publish exactly one V7 generation")

    digest = environment.digest(data)
    replay_text = uart.command(tracked["command"], "\n")
    replay = decode_operation_receipt(replay_text)
    if replay != tracked["receipt"] or _receipt_lines(replay_text) != tracked["lines"]:
        raise AssertionError("profile-1 tracked retry changed the old-format receipt")
    if environment.digest(data) != digest:
        raise AssertionError("exact profile-1 tracked retry changed the V7 image")

    profile2 = _profile2_write(uart, refs, lineage, tracked_version, state["epoch"], PROFILE2_KEY)
    profile2_live = _live(data)
    _records(profile2_live, SEEDED_RECORDS + 2, "profile-2 size-boundary commit")
    match_records(profile2_live, [profile2["receipt"]], *refs_ids)
    profile2_version = profile2["receipt"]["version"]
    _check_live_bytes(profile2_live, refs_ids[1], profile2_version, profile2["content"],
                      "profile-2 size-boundary commit")

    # The old, unmarked lookup must preserve its 1024-byte receipt bound;
    # the existing marked lookup must still retrieve the same larger receipt.
    digest = environment.digest(data)
    for command in (
        f"operation {profile2['receipt']['id']}",
        f"operation {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(PROFILE2_KEY)}",
    ):
        _single_error(uart.command(command, "\n"), "Size")
    if environment.digest(data) != digest:
        raise AssertionError("a refused profile-1 large-receipt lookup changed the V7 image")
    large_lookup = _profile2_lookup(uart, f"operation-v7 {profile2['receipt']['id']}", profile2,
                                    "profile-2 size-boundary lookup")
    if large_lookup["size"] != PROFILE2_SIZE:
        raise AssertionError("profile-2 lookup truncated a receipt above the profile-1 limit")
    if environment.digest(data) != digest:
        raise AssertionError("a profile-2 receipt lookup changed the V7 image")

    state["version"] = profile2_version
    file_before_admission = _file(profile2_live, refs_ids[1])
    accepted = _profile1_admit(uart, refs, state["version"], state["epoch"], EXECUTE_KEY, EXECUTE_CONTENT)
    accepted_snapshot = _live(data)
    _records(accepted_snapshot, SEEDED_RECORDS + 3, "profile-1 admission acceptance")
    admitted_record = _check_admission(
        accepted_snapshot, accepted["status"], refs_ids, state["epoch"], EXECUTE_KEY,
        state["version"], EXECUTE_CONTENT, "profile-1 admission acceptance"
    )
    if _file(accepted_snapshot, refs_ids[1]) != file_before_admission:
        raise AssertionError("profile-1 admission acceptance changed the live file")
    if accepted_snapshot["sequence"] != profile2_live["sequence"] + 1 \
            or accepted["status"]["number"] != accepted_snapshot["sequence"]:
        raise AssertionError("profile-1 admission did not publish one generation named by its ID")

    admission_retry = f"admission {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(EXECUTE_KEY)}"
    admission_id = f"admission {accepted['status']['id']}"
    admission_replay = _same_admission_status(
        uart, [accepted["command"], admission_retry, admission_id], accepted, data,
        "profile-1 exact admission retry and lookups"
    )
    after_replays = _live(data)
    _records(after_replays, SEEDED_RECORDS + 3, "profile-1 admission retries")
    _check_admission(after_replays, accepted["status"], refs_ids, state["epoch"], EXECUTE_KEY,
                     state["version"], EXECUTE_CONTENT, "profile-1 admission retry")

    return {
        "tracked": tracked,
        "tracked_record": tracked_record,
        "profile2_boundary": {
            "receipt": profile2["receipt"], "profile1_id_lookup": "Size",
            "profile1_retry_lookup": "Size", "profile2_lookup": large_lookup["lines"],
        },
        "accepted": accepted,
        "admitted_record": admitted_record,
        "admission_replays": admission_replay,
        "after_shutdown": {"sequence": after_replays["sequence"], "records": len(after_replays["records"])},
        "record_counts": {
            "after_tracked": len(tracked_live["records"]),
            "after_profile2_boundary": len(profile2_live["records"]),
            "after_admission": len(after_replays["records"]),
        },
        "state": dict(state),
    }


def _check_completion_receipt(receipt, refs, refs_ids, lineage, status, previous, epoch, key, content):
    check_operation_receipt(receipt, refs["workspace"], refs["resource"], previous, epoch, key, content)
    # The operation has the terminal sequence as its committed version, while
    # the admission ID and service instance preserve the accepting generation.
    operation = receipt["receipt"]
    version = int(operation["version"][2:], 16)
    expected_id = f"op_{lineage}_{version:016x}"
    if receipt["operation_id"] != expected_id or status["completion"] != expected_id:
        raise AssertionError("profile-1 completion receipt and admission name different operations")
    if operation["version"] != f"v_{status['terminal']:016x}":
        raise AssertionError("profile-1 completion receipt version differs from the admission terminal")
    if receipt["service_instance"] != status["instance"]:
        raise AssertionError("profile-1 completion receipt lost the admission service instance")
    if operation["workspace"] != refs["workspace"] or operation["resource"] != refs["resource"]:
        raise AssertionError("profile-1 completion receipt names another V7 object")
    if operation["retry"] != {"epoch": f"e_{epoch:016x}", "key": f"k_{key:016x}"}:
        raise AssertionError("profile-1 completion receipt changed the admission retry key")
    if int(refs["workspace"][36:], 16) != refs_ids[0] or int(refs["resource"][-8:], 16) != refs_ids[1]:
        raise AssertionError("profile-1 completion receipt refs differ from the provisioned identities")
    if not receipt["service_instance"].startswith(f"si_{lineage}_"):
        raise AssertionError("profile-1 completion receipt names another lineage")
    return version


def _second_boot(uart, data, refs, refs_ids, lineage, preboot_digest, first):
    tracked = first["tracked"]
    profile2 = first["profile2_boundary"]
    accepted = first["accepted"]
    state = dict(first["state"])
    before = _live(data)
    _records(before, SEEDED_RECORDS + 3, "boot-2 retained records")
    if environment.digest(data) != preboot_digest:
        raise AssertionError("the second V7 boot changed the image before any native command")
    # These are cold profile-1 ID and retry-key lookups after reboot. Their
    # exact legacy receipt lines must equal both the commit and exact replay.
    digest = environment.digest(data)
    profile1_id = _profile1_operation(
        uart, f"operation {tracked['receipt']['operation_id']}", tracked, "cold profile-1 ID lookup"
    )
    profile1_retry = _profile1_operation(
        uart,
        f"operation {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(TRACKED_KEY)}",
        tracked,
        "cold profile-1 retry lookup",
    )
    # Both profile-1 spellings must refuse the retained 1025-byte receipt.
    for command in (
        f"operation {profile2['receipt']['id']}",
        f"operation {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(PROFILE2_KEY)}",
    ):
        _single_error(uart.command(command, "\n"), "Size")
    p2_id = _profile2_lookup(uart, f"operation-v7 {profile2['receipt']['id']}",
                             {"lines": profile2["receipt"]["lines"]}, "cold profile-2 ID lookup")
    p2_retry = _profile2_lookup(
        uart, f"operation-v7 {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(PROFILE2_KEY)}",
        {"lines": profile2["receipt"]["lines"]}, "cold profile-2 retry lookup"
    )
    if p2_id["lines"] != p2_retry["lines"]:
        raise AssertionError("cold profile-2 lookups by ID and retry key differ")
    if environment.digest(data) != digest:
        raise AssertionError("cold receipt lookups changed the V7 image")

    replay_text = uart.command(tracked["command"], "\n")
    replay_receipt = decode_operation_receipt(replay_text)
    if replay_receipt != tracked["receipt"] or _receipt_lines(replay_text) != tracked["lines"]:
        raise AssertionError("rebooted profile-1 exact retry changed the old-format receipt")
    if environment.digest(data) != digest:
        raise AssertionError("rebooted profile-1 exact retry changed the V7 image")

    admission_retry = f"admission {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(EXECUTE_KEY)}"
    admission_id = f"admission {accepted['status']['id']}"
    _same_admission_status(uart, [admission_id, admission_retry], accepted, data,
                           "cold profile-1 admission lookups")

    committed = _status_command(uart, f"execute-admission {accepted['status']['id']}")
    if committed["state"] != "committed" or committed["id"] != accepted["status"]["id"] \
            or committed["instance"] != accepted["status"]["instance"]:
        raise AssertionError(f"profile-1 admitted operation did not execute under its retained identity: {committed}")
    after_execute = _live(data)
    _records(after_execute, SEEDED_RECORDS + 3, "profile-1 admission execution")
    executed_record = _check_admission(
        after_execute, committed, refs_ids, state["epoch"], EXECUTE_KEY,
        state["version"], EXECUTE_CONTENT, "profile-1 admission execution"
    )
    executed_version = committed["terminal"]
    _check_live_bytes(after_execute, refs_ids[1], executed_version, EXECUTE_CONTENT,
                      "profile-1 admission execution")
    if after_execute["sequence"] != before["sequence"] + 1:
        raise AssertionError("profile-1 admission execution did not publish exactly one generation")

    completion_id = committed["completion"]
    completion_by_id_text = uart.command(f"operation {completion_id}", "\n")
    completion_by_id = decode_operation_receipt(completion_by_id_text)
    completion_version = _check_completion_receipt(
        completion_by_id, refs, refs_ids, lineage, committed, state["version"], state["epoch"],
        EXECUTE_KEY, EXECUTE_CONTENT
    )
    completion_lines = _receipt_lines(completion_by_id_text)
    completion_by_retry_text = uart.command(
        f"operation {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(EXECUTE_KEY)}", "\n"
    )
    completion_by_retry = decode_operation_receipt(completion_by_retry_text)
    if completion_by_retry != completion_by_id or _receipt_lines(completion_by_retry_text) != completion_lines:
        raise AssertionError("profile-1 completion lookups by ID and retry key differ")
    if executed_record["state"] != "admitted_committed" or executed_record["terminal"] != completion_version:
        raise AssertionError("profile-1 completion receipt does not match the admitted committed record")
    committed_replays = _same_admission_status(
        uart,
        [f"admission {committed['id']}",
         f"admission {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(EXECUTE_KEY)}",
         f"execute-admission {committed['id']}", f"cancel-admission {committed['id']}"],
        {"status": committed}, data, "profile-1 committed admission replays"
    )

    # Accept and explicitly cancel a second profile-1 admission. The live file
    # remains the first executed admission while its cancellation is retained.
    state["version"] = completion_version
    file_before_cancel = _file(after_execute, refs_ids[1])
    pending_cancel = _profile1_admit(
        uart, refs, state["version"], state["epoch"], CANCEL_KEY, CANCEL_CONTENT
    )
    after_accept_cancel = _live(data)
    _records(after_accept_cancel, SEEDED_RECORDS + 4, "profile-1 cancellation admission")
    _check_admission(after_accept_cancel, pending_cancel["status"], refs_ids, state["epoch"], CANCEL_KEY,
                     state["version"], CANCEL_CONTENT, "profile-1 cancellation admission")
    if _file(after_accept_cancel, refs_ids[1]) != file_before_cancel:
        raise AssertionError("accepting the profile-1 cancellation case changed the live file")

    cancelled = _status_command(uart, f"cancel-admission {pending_cancel['status']['id']}")
    if cancelled["state"] != "cancelled" or cancelled["number"] != pending_cancel["status"]["number"]:
        raise AssertionError(f"profile-1 admission cancellation did not report cancelled: {cancelled}")
    after_cancel = _live(data)
    _records(after_cancel, SEEDED_RECORDS + 4, "profile-1 admission cancellation")
    cancelled_record = _check_admission(
        after_cancel, cancelled, refs_ids, state["epoch"], CANCEL_KEY,
        state["version"], CANCEL_CONTENT, "profile-1 admission cancellation"
    )
    if cancelled_record["state"] != "cancelled" or _file(after_cancel, refs_ids[1]) != file_before_cancel:
        raise AssertionError("profile-1 cancellation changed the live file or retained state")
    if after_cancel["sequence"] != after_accept_cancel["sequence"] + 1:
        raise AssertionError("profile-1 cancellation did not publish exactly one generation")

    cancelled_expected = {"command": pending_cancel["command"], "status": cancelled}
    cancellation_statuses = _same_admission_status(
        uart,
        [f"admission {cancelled['id']}",
         f"admission {refs['workspace']} e_{hex16(state['epoch'])} k_{hex16(CANCEL_KEY)}",
         f"cancel-admission {cancelled['id']}",
         f"execute-admission {cancelled['id']}"],
        cancelled_expected, data, "profile-1 cancelled status and terminal replays"
    )
    final = _live(data)
    _records(final, SEEDED_RECORDS + 4, "profile-1 final retained table")
    if _file(final, refs_ids[1]) != file_before_cancel:
        raise AssertionError("profile-1 status replays changed the live file")

    return {
        "cold_profile1_id": profile1_id["lines"],
        "cold_profile1_retry": profile1_retry["lines"],
        "cold_profile2_id": p2_id["lines"],
        "cold_profile2_retry": p2_retry["lines"],
        "tracked_replay": replay_receipt,
        "accepted_status_after_reboot": accepted["status"]["lines"],
        "executed": committed,
        "executed_record": executed_record,
        "completion_receipt": completion_lines,
        "completion_lookup_id_retry_equal": True,
        "committed_replays": committed_replays,
        "cancel_admission": pending_cancel["status"],
        "cancelled": cancelled,
        "cancelled_record": cancelled_record,
        "cancellation_replays": cancellation_statuses,
        "record_counts": {
            "at_boot": len(before["records"]),
            "after_execution": len(after_execute["records"]),
            "after_cancellation_admission": len(after_accept_cancel["records"]),
            "after_cancellation": len(after_cancel["records"]),
            "after_replays": len(final["records"]),
        },
        "final": {"sequence": final["sequence"], "records": len(final["records"])},
    }


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-profile1")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    for path, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(path) != metadata["native_applications"]["file-server"][suffix]:
            raise RuntimeError(f"{path.name} differs from the artifact recorded by the boot build")
    (output / "result.json").unlink(missing_ok=True)
    started = time.monotonic()
    phases = (1, 2)
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-v7-profile1-") as temporary:
            temporary = Path(temporary)
            data = temporary / "volume.raw"
            lineage = uuid.uuid4().hex
            seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
            scratch = seeded["scratch"]
            refs = {"workspace": seeded["workspace"]["text"], "resource": scratch["resource"]}
            refs_ids = (seeded["workspace"]["id"], scratch["id"])
            initial = _live(data)
            _records(initial, SEEDED_RECORDS, "fresh seed")
            if initial["lineage"] != lineage:
                raise AssertionError("seed7 created a volume with another lineage")
            state = {"epoch": initial["epoch"], "version": scratch["version"]}
            first = boot_terminal(
                image, data, output, 1, temporary,
                lambda uart: _first_boot(uart, data, refs, refs_ids, lineage, state),
            )
            after_first = _live(data)
            _records(after_first, SEEDED_RECORDS + 3, "profile-1 boot-1 shutdown")
            if after_first["sequence"] != first["after_shutdown"]["sequence"]:
                raise AssertionError("boot-1 shutdown changed the selected V7 sequence")
            before_second = environment.digest(data)
            report = volume_json(volume_tool, "report7", data)
            if report["sequence"] != after_first["sequence"] or len(report["records"]) != len(after_first["records"]):
                raise AssertionError("rustic-volume report7 and independent oracle7 disagree after boot 1")

            second = boot_terminal(
                image, data, output, 2, temporary,
                lambda uart: _second_boot(uart, data, refs, refs_ids, lineage, before_second, first),
            )
            final = _live(data)
            _records(final, SEEDED_RECORDS + 4, "profile-1 boot-2 shutdown")
            report = volume_json(volume_tool, "report7", data)
            if report["sequence"] != final["sequence"] or len(report["records"]) != len(final["records"]):
                raise AssertionError("rustic-volume report7 and independent oracle7 disagree after boot 2")
            tracked_record = _check_profile1_record(final, refs_ids, first["tracked"]["receipt"], TRACKED_CONTENT,
                                                    "final retained profile-1 tracked write")
            p2_receipt = first["profile2_boundary"]["receipt"]
            match_records(final, [p2_receipt], *refs_ids)
            _check_admission(final, second["executed"], refs_ids, first["state"]["epoch"], EXECUTE_KEY,
                             first["state"]["version"], EXECUTE_CONTENT,
                             "final retained profile-1 executed admission")
            _check_admission(final, second["cancelled"], refs_ids, first["state"]["epoch"], CANCEL_KEY,
                             second["executed"]["terminal"], CANCEL_CONTENT,
                             "final retained profile-1 cancelled admission")
            _check_live_bytes(final, refs_ids[1], second["executed"]["terminal"], EXECUTE_CONTENT,
                              "final profile-1 live file")
            if final["lineage"] != lineage:
                raise AssertionError("reboot changed the V7 lineage")

        evidence = {
            "verified": True,
            "mode": "terminal-v7",
            "boots": 2,
            "lineage": lineage,
            "subject": SHELL_SUBJECT,
            "scratch": scratch,
            "initial_records": len(initial["records"]),
            "boot_1": first,
            "boot_2": second,
            "tracked_record_final": tracked_record,
            "record_counts": {
                "initial": len(initial["records"]),
                "boot_1": len(after_first["records"]),
                "final": len(final["records"]),
                "limit": oracle7.RETAINED,
            },
            "final_sequence": final["sequence"],
        }
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "terminal_v7_profile1": evidence,
        }
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print("V7 profile-1 native acceptance: existing tracked receipt/retry and admission accept/retry/execute/cancel "
              "paths matched their exact legacy SDK output; oracle7 confirmed retained bytes and bounded record counts "
              "across reboot. A 1025-byte profile-2 receipt remained available through operation-v7 while profile-1 "
              "lookups returned Size.", flush=True)
        return result
    finally:
        for kind in ("serial", "qemu"):
            (output / f"{kind}.log").write_bytes(b"\n".join(
                (output / f"{kind}-{phase}.log").read_bytes()
                for phase in phases if (output / f"{kind}-{phase}.log").exists()
            ))
