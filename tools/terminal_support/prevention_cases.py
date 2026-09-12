# SPDX-License-Identifier: Apache-2.0
"""Owner migration and retained causes: native IPC plus an independent disk reader.

Public profile 2 exposes causes; profile 1 retains its coarse compatible view.
"""
from .admission_cases import status
from .operation_cases import references
from .oracle import snapshot
from .scheduling_cases import settled
from . import prevention_observations as observations


def inspect(data, results, reasons):
    _, state = snapshot(data)
    if state["format"] != 5 or len(state["records"]) != len(results):
        raise AssertionError("incorrect migrated format or retained inventory")
    for result, reason in zip(results, reasons, strict=True):
        records = [r for r in state["records"] if r.get("admission") == result["number"]]
        if len(records) != 1:
            raise AssertionError("missing unique persisted prevention")
        record = records[0]
        if (record["state"], record["terminal"], record["prevention"], record["committed"]) != (
                "cancelled", result["terminal"], reason, 0):
            raise AssertionError("public prevention disagrees with exact disk cause")
    return state


def exercise(uart, data):
    uart.command("enable-operations", "persistent format v3")
    uart.command("enable-admissions", "persistent format v4")
    _, initial = snapshot(data)
    if initial["records"]:
        raise AssertionError("prevention fixture needs empty retention slots")
    ws, resource = references(uart, ".", "hello")
    file_id = int(resource.rsplit("_", 1)[1], 16)
    version = initial["nodes"][file_id]["version"]

    def admit(epoch, key):
        return status(uart.command(
            f'admit-ref {ws} {resource} v_{version:016x} e_{epoch:016x} k_{key:016x} "never"'))

    old = admit(initial["epoch"], 0x8000)
    old = status(uart.command(f"cancel-admission {old['id']}"))
    legacy_view = observations.paired_retained(uart, [old], ['unknown'])
    if snapshot(data)[1]['format'] != 4:
        raise AssertionError('profile 2 silently migrated legacy storage')
    uart.command("enable-prevention-reasons", "persistent format v5")
    legacy = inspect(data, [old], ["unknown"])
    selected = snapshot(data)[0]
    if (selected[8 * 512 + 8], selected[13 * 512 + 8]) != (5, 5):
        raise AssertionError("migration acknowledged before both banks upgraded")
    if legacy["nodes"][file_id]["version"] != version or legacy["files"] != initial["files"]:
        raise AssertionError("legacy prevention or migration changed the file")
    uart.command("enable-prevention-reasons", "persistent format v5")
    if status(uart.command(f"cancel-admission {old['id']}")) != old:
        raise AssertionError("migration or replay relabeled a legacy record")
    if snapshot(data)[1]["selected_sha256"] != legacy["selected_sha256"]:
        raise AssertionError("completed migration or terminal replay wrote storage")
    migrated_view = observations.paired_retained(uart, [old], ['unknown'])
    if legacy_view != migrated_view:
        raise AssertionError('migration relabeled public legacy prevention')
    uart.command('rotate-receipts')
    authority, authority_view = observations.authority(uart, data, admit(snapshot(data)[1]['epoch'], 0x8003))
    authority_state = inspect(data, [authority], ['authority_lost'])
    if authority_state['files'] != initial['files'] or authority_state['nodes'][file_id]['version'] != version:
        raise AssertionError('authority loss did not prevent the file effect')
    uart.command("rotate-receipts")
    epoch = snapshot(data)[1]["epoch"]
    requested, conflict = (admit(epoch, key) for key in (0x8001, 0x8002))
    prepared_view = observations.paired_retained(uart, [requested, conflict], ['none', 'none'])
    requested = status(uart.command(f"cancel-admission {requested['id']}"))
    before_edit = snapshot(data)[1]
    if before_edit["nodes"][file_id]["version"] != version or before_edit["files"] != initial["files"]:
        raise AssertionError("requested prevention changed the file")
    # Same bytes, new version: the old expected version must still prevent execution.
    uart.command('write hello "Hello from native Rust"')
    edited_version = snapshot(data)[1]["nodes"][file_id]["version"]
    if edited_version <= version:
        raise AssertionError("conflict fixture did not change the version")
    uart.command(f"schedule-admission {conflict['id']}", "phase=queued")
    conflict = settled(uart, conflict)
    results, reasons = [requested, conflict], ["requested", "version_conflict"]
    final = inspect(data, results, reasons)
    if final["files"] != initial["files"] or final["nodes"][file_id]["version"] != edited_version:
        raise AssertionError("prevented work changed file bytes or leaked temporary files")
    terminal_view = observations.paired_retained(uart, results, reasons)
    return {"verified": True, "format": 5, "legacy_cause": "unknown",
            "replay_writes": 0, "results": results, "reasons": reasons,
            "observations": dict(legacy=legacy_view, migrated=migrated_view, authority=authority_view,
                                 prepared=prepared_view, terminal=terminal_view),
            "selected_sha256": final["selected_sha256"]}


def after_reboot(uart, data, before):
    for expected in before["results"]:
        if status(uart.command(f"admission {expected['id']}")) != expected:
            raise AssertionError("reboot changed a terminal prevention")
    before['observations']['reboot'] = observations.paired_retained(uart, before['results'], before['reasons'])
    if before['observations']['reboot'] != before['observations']['terminal']:
        raise AssertionError('reboot changed a public cause or client result')
    observed = inspect(data, before["results"], before["reasons"])
    if observed["selected_sha256"] != before["selected_sha256"]:
        raise AssertionError("reboot or inspection modified retained causes")


def checkpoint(data, evidence):
    """Other terminal scenarios may mutate unrelated metadata before shutdown."""
    state = inspect(data, evidence["results"], evidence["reasons"])
    evidence["selected_sha256"] = state["selected_sha256"]
