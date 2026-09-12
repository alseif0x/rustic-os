# SPDX-License-Identifier: Apache-2.0
"""Declared correspondence from the native live-control profile to service-v1 operations.

This maps observed native facts onto the reviewed `operation` type. It does not
implement `operations.get`/`operations.cancel` on the guest and claims no adapter
conformance. Its purpose is to reject a mapping that would report false success,
a rollback that did not happen, or a result under stale authority.
"""
import re
from .validation import ContractError, require, validate_exchange
from .activity_evidence import ACTIVITY_CASES, check_activity
from .activity_identity import native_identity, observation_flags

# Volatile observation phases. A live observation is never a terminal result:
# `settling` cannot claim effect none, and no phase may map to succeeded/cancelled.
LIVE = {"queued": ("queued", "none"), "running": ("running", "none"), "stopping": ("running", "none"),
        "settling": ("reconciling", "unknown")}

# Retained admission records, read after settlement through the durable API.
RECORDS = {"cancelled": ("cancelled", "none"),
           "committed": ("succeeded", "committed")}

# Native refusals of a live request. `outcome_unknown` keeps a hidden identity
# hidden: the caller learns its own knowledge state, not whether the work exists.
DENIALS = {17: ("access_denied", "stop", "none"), 18: ("access_denied", "stop", "none"),
           19: ("access_denied", "stop", "none"), 27: ("outcome_unknown", "reconcile", "unknown"),
           31: ("unavailable", "retry_same", "none")}


def request(method, params): return {"version": 1, "method": method, "params": params}


def response(method, result): return {"version": 1, "method": method, "result": result}


def identity(status):
    """Pre-terminal work is identified by its admission; a committed result is
    identified by the retained completed operation. The native profile does not
    yet carry one stable operation identity across that boundary."""
    lineage, number = native_identity(status)
    terminal = status.get("terminal")
    require(type(terminal) is int and 0 <= terminal <= 2**64 - 1, "missing terminal marker")
    require(status.get("state") in ("admitted", "cancelled", "committed"), "unknown retained state")
    # Terminal is zero only while the request is still admitted.
    require((terminal == 0) == (status.get("state") == "admitted"),
            "terminal marker contradicts the retained state")
    require(terminal == 0 or terminal > number, "terminal precedes its admission")
    if status.get("state") == "committed":
        return f"op_{lineage}_{terminal:016x}", status["instance"]
    return status["id"], status["instance"]


def live_operation(status, observation):
    """One volatile observation as a service-v1 operation."""
    require(isinstance(observation, dict) and observation.get("phase") in LIVE,
            "unknown live phase")
    native_identity(status)
    require(observation.get("id") == status.get("id")
            and observation.get("instance") == status.get("instance"),
            "observation belongs to another admission or service instance")
    requested = observation_flags(observation)
    state, effect = LIVE[observation["phase"]]
    # An accepted stop is a request, never prevention: the operation stays running.
    return {"operation_id": status["id"], "service_instance": status["instance"],
            "cancel_requested": requested, "state": state, "effect": effect}


def record_operation(status, requested):
    """The retained record after settlement. `succeeded` carries its receipt only
    through the completed-operation API, which the operations profile checks."""
    require(status.get("state") in RECORDS, "prepared admission has no scheduled service-v1 state")
    require(type(requested) is bool, "invalid cancellation history")
    state, effect = RECORDS[status["state"]]
    operation_id, instance = identity(status)
    result = {"operation_id": operation_id, "service_instance": instance,
              "cancel_requested": bool(requested), "state": state, "effect": effect}
    return result, state == "succeeded"


def uncertain_operation(status, requested=False):
    """A lost or failed settlement leaves the caller reconciling, whatever the
    record currently says. Never report the retained state as the attempt result."""
    require(type(requested) is bool, "invalid cancellation history")
    operation_id, instance = identity(status)
    return {"operation_id": operation_id, "service_instance": instance,
            "cancel_requested": bool(requested), "state": "reconciling", "effect": "unknown"}


def denial(code):
    require(type(code) is int and code in DENIALS, "unmapped native refusal")
    fields = DENIALS[code]
    return {"code": fields[0], "next_action": fields[1], "effect": fields[2]}


def check_operation(catalog, operation):
    validate_exchange(catalog, request("operations.get", {"operation_id": operation["operation_id"]}),
                      response("operations.get", operation))


def check_case(catalog, case):
    """Check one native live-control case end to end against the contract."""
    require(isinstance(case, dict) and case.get("verified") is True, "unverified native activity case")
    status = case.get("durable")
    require(isinstance(status, dict), "case without a durable admission status")
    for field in ("uncertain", "stopped"):
        require(type(case.get(field, False)) is bool, "invalid case flag")
    observations = case.get("observations", [])
    require(isinstance(observations, list) and (observations or case.get("uncertain")),
            "missing live observations")
    observed = False
    last_phase = -1
    for observation in observations:
        operation = live_operation(status, observation)
        check_operation(catalog, operation)
        phase = ("queued", "running", "stopping", "settling").index(observation["phase"])
        require(phase >= last_phase, "publication phase moved backwards")
        last_phase = phase
        require(not observed or operation["cancel_requested"],
                "an accepted stop request was later withdrawn")
        observed = observed or operation["cancel_requested"]
        require(operation["state"] != "cancelled" and operation["effect"] != "committed",
                "a volatile observation claimed a terminal result")
    require(not observations or any(o["pending"] == 1 for o in observations),
            "live control was never observed during pending I/O")
    # A stop accepted through another client is recorded by the case itself.
    requested = observed or bool(case.get("stopped"))
    if case.get("uncertain"):
        operation = uncertain_operation(status, requested)
        check_operation(catalog, operation)
        require(operation["effect"] == "unknown", "an uncertain attempt claimed a known effect")
        return operation
    settled, committed = record_operation(status, requested)
    if committed:
        completion = case.get("completion")
        require(isinstance(completion, dict), "committed case lacks its own completion receipt")
        check_operation(catalog, completion)
        require(completion.get("state") == "succeeded"
                and completion.get("operation_id") == settled["operation_id"]
                and completion.get("service_instance") == settled["service_instance"],
                "completion belongs to another admission or service instance")
        settled["receipt"] = completion["receipt"]
    check_operation(catalog, settled)
    require(case.get("committed") is committed, "record contradicts the reported effect")
    last = observations[-1]["phase"] if observations else None
    require(last != "settling" or settled["state"] != "cancelled",
            "a settling publication reported a rollback")
    require(settled["state"] != "cancelled" or requested,
            "prevention without an accepted stop request")
    if case.get("denied"):
        require(denial(case["denied"])["code"] in ("access_denied", "outcome_unknown")
                and settled["state"] == "succeeded",
                "a refused stop changed the outcome")
    return settled


def native_check(catalog, evidence):
    require(isinstance(evidence, dict) and evidence.get("verified") is True, "unverified evidence")
    require(isinstance(evidence.get("kernel_sha256"), str)
            and re.fullmatch(r"[0-9a-f]{64}", evidence["kernel_sha256"]), "missing guest identity")
    inventory = evidence.get("cases")
    require(isinstance(inventory, list) and all(isinstance(c, dict) for c in inventory),
            "invalid live-control inventory")
    cases = [c for c in inventory if str(c.get("case", "")).startswith("public_activity_")]
    require(len(cases) == len(ACTIVITY_CASES) and {c["case"] for c in cases} == ACTIVITY_CASES,
            "missing, duplicate or unknown live-control case")
    check_activity(cases)
    states = {}
    for case in cases:
        states[case["case"]] = check_case(catalog, case)["state"]
    require(set(states.values()) >= {"succeeded", "cancelled", "reconciling"},
            "the mapping never exercised success, prevention and reconciliation")
    return {"status": "success", "backend": "native_uart_activity_evidence",
            "mapped_operations": ["operations.get"], "unimplemented_methods": ["operations.cancel"],
            "profile": "live_control_correspondence", "cases": len(cases),
            "states": states, "guest_execution": True,
            "kernel_sha256": evidence["kernel_sha256"]}


def host_check(catalog):
    """Enumerate the declared correspondence, including the vectors it must reject."""
    status = {"id": "ad_" + "07" * 16 + "_0000000000000003", "lineage": "07" * 16,
              "instance": "si_" + "07" * 16 + "_0000000000000003", "state": "admitted", "terminal": 0}
    accepted = 0
    for phase, requested in (("running", 0), ("stopping", 1), ("settling", 0), ("settling", 1)):
        observation = {"id": status["id"], "instance": status["instance"], "phase": phase,
                       "requested": requested, "pending": 1}
        check_operation(catalog, live_operation(status, observation))
        accepted += 1
    rejected = 0
    for observation in ({"id": status["id"], "instance": status["instance"], "phase": "cancelled",
                         "requested": 1, "pending": 0},
                        {"id": status["id"], "instance": status["instance"], "phase": "stopping",
                         "requested": 0, "pending": 1},
                        {"id": "ad_" + "1" * 32 + "_0000000000000003", "instance": status["instance"],
                         "phase": "running", "requested": 0, "pending": 1}):
        try:
            live_operation(status, observation)
        except ContractError:
            rejected += 1
    require(rejected == 3, "the mapping accepted an incoherent observation")
    try:
        record_operation(status, False)
        raise AssertionError("preparation was mistaken for scheduled execution")
    except ContractError:
        rejected += 1
    for state, terminal in (("cancelled", 8), ("committed", 9)):
        operation, committed = record_operation({**status, "state": state, "terminal": terminal}, False)
        if committed:
            # Deliberately incomplete here: the contract rejects `succeeded` without
            # its receipt, which only the completed-operation API can supply.
            try:
                check_operation(catalog, operation)
                raise AssertionError("succeeded was accepted without a receipt")
            except ContractError:
                rejected += 1
        else:
            check_operation(catalog, operation)
        accepted += 1
    check_operation(catalog, uncertain_operation(status))
    for code in DENIALS:
        require(set(denial(code)) == {"code", "next_action", "effect"}, "invalid mapped refusal")
        validate_exchange(catalog, request("operations.get", {"operation_id": status["id"]}),
                          {"version": 1, "method": "operations.get", "error": denial(code)})
    return {"status": "success", "backend": "declared_activity_correspondence",
            "mapped_operations": ["operations.get"], "unimplemented_methods": ["operations.cancel"],
            "profile": "live_control_correspondence", "accepted_vectors": accepted,
            "rejected_vectors": rejected, "refusals": len(DENIALS), "guest_execution": False}
