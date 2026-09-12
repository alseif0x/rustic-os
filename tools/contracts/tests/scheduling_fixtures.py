# SPDX-License-Identifier: Apache-2.0
"""Synthetic contract challenges only; these objects are not native evidence."""
import hashlib


def scheduling_cases():
    cases = []
    definitions = (
        ("scheduled_queue", ("committed", "cancelled"), ((0, "queued", 0, 0), (0, "running", 1, 0), (1, "queued", 0, 0), (1, "queued", 0, 1))),
        ("scheduled_lost_reply", ("cancelled",), ((0, "running", 1, 0), (0, "stopping", 1, 1))),
        ("scheduled_restart", ("admitted", "committed"), ((0, "queued", 0, 0), (0, "running", 1, 0), (1, "queued", 0, 0), (1, "queued", 0, 0))),
        ("scheduled_revoked", ("committed", "cancelled"), ((0, "queued", 0, 0), (0, "running", 1, 0), (1, "queued", 0, 0))),
    )
    for name, states, phases in definitions:
        lineage = "07" * 16
        instance = f"si_{lineage}_0000000000000003"
        durable = [dict(id=f"ad_{lineage}_{i + 3:016x}", lineage=lineage, number=i+3,
                        instance=instance, state=state, terminal=0 if state == "admitted" else i+5)
                   for i, state in enumerate(states)]
        observations = [dict(id=durable[i]["id"], instance=instance, phase=phase, pending=pending, requested=requested)
                        for i, phase, pending, requested in phases]
        case = dict(case=name, verified=True, reboot_verified=True, sha256="a" * 64,
                    durable=durable, observations=observations,
                    peer_ack={"value": 4, "other": 0, "control_denied": 0})
        case.update({flag: True for flag in ("retained_full", "duplicate_same", "cancel_only", "owner_progress",
                    "discarded_reply", "stale_reply_rejected", "no_replay", "fresh_explicit", "revoked_before_execution")})
        if "committed" in states:
            status = durable[states.index("committed")]
            data = b"second" if name == "scheduled_restart" else b"first"
            case["completion"] = dict(operation_id=f"op_{lineage}_{status['terminal']:016x}",
                service_instance=instance, state="succeeded", effect="committed", cancel_requested=False,
                receipt=dict(workspace="workspace_a", resource="file_a", previous_version="v_1",
                             version=f"v_{status['terminal']:016x}", size=len(data), sha256=hashlib.sha256(data).hexdigest(),
                             retry=dict(epoch="epoch_a", key="key_a")))
        if name == "scheduled_restart":
            case["recovered"] = [{**status, "state": "admitted", "terminal": 0} for status in durable]
        cases.append(case)
    return cases
