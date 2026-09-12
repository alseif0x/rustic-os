# SPDX-License-Identifier: Apache-2.0
"""Native queue evidence must distinguish scheduling, I/O and retained outcomes."""
import hashlib
import re
from .validation import require
from .activity_conformance import identity, live_operation, check_operation

NAMES = ("scheduled_queue", "scheduled_lost_reply", "scheduled_restart", "scheduled_revoked")
STATES = (("committed", "cancelled"), ("cancelled",), ("admitted", "committed"), ("committed", "cancelled"))
OBSERVATIONS = (
    ((0, "queued", 0, 0), (0, "running", 1, 0), (1, "queued", 0, 0), (1, "queued", 0, 1)),
    ((0, "running", 1, 0), (0, "stopping", 1, 1)),
    ((0, "queued", 0, 0), (0, "running", 1, 0), (1, "queued", 0, 0), (1, "queued", 0, 0)),
    ((0, "queued", 0, 0), (0, "running", 1, 0), (1, "queued", 0, 0)),
)


def check_scheduling(catalog, cases):
    found = [c for c in cases if isinstance(c, dict) and str(c.get("case", "")).startswith("scheduled_")]
    require(len(found) == len(NAMES) and {c["case"] for c in found} == set(NAMES),
            "missing, duplicate or unknown scheduling case")
    for name, states, expected in zip(NAMES, STATES, OBSERVATIONS):
        case = next(c for c in found if c["case"] == name)
        require(case.get("verified") is True and case.get("reboot_verified") is True,
                "unverified scheduling or reboot")
        require(isinstance(case.get("sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", case["sha256"]),
                "missing independent scheduled disk identity")
        durable = case.get("durable")
        require(isinstance(durable, list) and len(durable) == len(states)
                and all(isinstance(s, dict) for s in durable), "invalid scheduled outcomes")
        require(tuple(s.get("state") for s in durable) == states, "scheduled outcome contradicts the cut")
        for status in durable:
            identity(status)
        require(len({s["id"] for s in durable}) == len(durable), "different queue tickets alias one admission")
        observed = case.get("observations")
        require(isinstance(observed, list) and len(observed) == len(expected), "incomplete scheduled observations")
        for actual, (index, phase, pending, requested) in zip(observed, expected):
            mapped = live_operation(durable[index], actual)
            check_operation(catalog, mapped)
            require((actual["phase"], actual["pending"], actual["requested"]) == (phase, pending, requested),
                    "observation does not establish the specified scheduling boundary")
        if "committed" in states:
            status = durable[states.index("committed")]
            completion = case.get("completion")
            require(isinstance(completion, dict), "scheduled success lacks its own receipt")
            check_operation(catalog, completion)
            require((completion.get("operation_id"), completion.get("service_instance")) == identity(status)
                    and completion.get("state") == "succeeded", "scheduled receipt belongs to other work")
            data = b"second" if name == "scheduled_restart" else b"first"
            receipt = completion["receipt"]
            require(receipt["sha256"] == hashlib.sha256(data).hexdigest() and receipt["size"] == len(data)
                    and receipt["version"] == f"v_{status['terminal']:016x}", "scheduled receipt differs from expected content/version")
        if name in ("scheduled_queue", "scheduled_revoked"):
            peer = case.get("peer_ack")
            require(isinstance(peer, dict) and all(type(peer.get(field)) is int and peer[field] == value
                    for field, value in (("value", 4), ("other", 0), ("control_denied", 0))),
                    "deterministic client did not acknowledge the same queued state")
        flags = {"scheduled_queue": ("retained_full", "duplicate_same", "cancel_only", "owner_progress"),
                 "scheduled_lost_reply": ("discarded_reply", "stale_reply_rejected"),
                 "scheduled_restart": ("no_replay", "fresh_explicit"),
                 "scheduled_revoked": ("revoked_before_execution",)}[name]
        require(all(case.get(flag) is True for flag in flags), "missing scheduling authority/recovery evidence")
        if name == "scheduled_restart":
            recovered = case.get("recovered")
            require(isinstance(recovered, list) and len(recovered) == len(durable)
                    and all(isinstance(s, dict) for s in recovered), "missing retained facts after restart")
            for old, final in zip(recovered, durable):
                identity(old)
                require(old["state"] == "admitted" and old["id"] == final["id"]
                        and old["instance"] == final["instance"], "restart replayed or replaced queued work")
    return len(found)
