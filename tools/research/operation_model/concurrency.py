# SPDX-License-Identifier: Apache-2.0
"""Enumerate one writer's steps interleaved with an owner edit and delegation."""
from itertools import permutations


def schedules():
    events = ("observe", "authorize", "commit", "human_edit", "revoke", "regrant")
    for order in permutations(events):
        if (order.index("observe") < order.index("authorize") < order.index("commit")
                and order.index("revoke") < order.index("regrant")):
            yield order


def execute(order, strategy):
    version, epoch, allowed = 0, 0, True
    observed, admitted_epoch, admitted = None, None, False
    committed, violations = False, []
    for event in order:
        if event == "observe":
            observed = version
        elif event == "authorize":
            admitted, admitted_epoch = allowed, epoch
        elif event == "human_edit":
            version += 1
        elif event == "revoke":
            epoch, allowed = epoch + 1, False
        elif event == "regrant":
            epoch, allowed = epoch + 1, True
        elif event == "commit":
            accept = admitted
            if strategy in ("version_only", "version_and_authority"):
                accept = accept and observed == version
            if strategy == "version_and_authority":
                accept = accept and allowed and admitted_epoch == epoch
            if accept:
                # Independent observation of the commit's pre-state. The final
                # strategy ASSUMES validation and effect are one atomic step.
                if observed != version:
                    violations.append("overwrote_intervening_edit")
                if not allowed or admitted_epoch != epoch:
                    violations.append("used_obsolete_delegation")
                version += 1
                committed = True
    return committed, violations


def explore():
    results = {}
    for strategy in ("admission_only", "version_only", "version_and_authority"):
        count, commits, invalid, examples = 0, 0, 0, {}
        for order in schedules():
            committed, violations = execute(order, strategy)
            count += 1
            commits += int(committed)
            invalid += int(bool(violations))
            for violation in violations:
                examples.setdefault(violation, list(order))
        results[strategy] = {"schedules": count, "commits": commits,
                             "invalid_schedules": invalid, "counterexamples": examples}
    return results
