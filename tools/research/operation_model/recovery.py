# SPDX-License-Identifier: Apache-2.0
"""Crash-cut model of one local effect and its deduplication receipt.

Python state here represents hypothetical durable state. No disk is accessed.
"""


def execute(cut, atomic_receipt):
    effects, receipt, replied = 0, False, False
    for step in ("prepare", "effect", "receipt", "reply")[:cut]:
        if step == "effect":
            effects += 1
            if atomic_receipt:
                receipt = True
        elif step == "receipt":
            receipt = True
        elif step == "reply":
            replied = True
    # Crash: volatile state is lost. If no reply arrived, repeat the SAME key
    # and arguments within retention. A durable receipt suppresses reexecution.
    if not replied and not receipt:
        effects += 1
        receipt = True
    return effects


def explore():
    results = {}
    for atomic in (False, True):
        failures = []
        for cut in range(5):
            effects = execute(cut, atomic)
            if effects != 1:
                failures.append({"crash_after_steps": cut, "effects": effects})
        results["atomic_effect_receipt" if atomic else "separate_effect_receipt"] = {
            "crash_cuts": 5, "invalid_cuts": len(failures), "counterexamples": failures}
    return results
