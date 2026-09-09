# SPDX-License-Identifier: Apache-2.0
"""Print reproducible evidence; exit nonzero if an expected distinction is lost."""
import json

from . import concurrency, recovery


def main():
    orders, crashes = concurrency.explore(), recovery.explore()
    checks = {
        "all_orders_enumerated": all(r["schedules"] == 60 for r in orders.values()),
        "admission_only_detects_both_faults": set(orders["admission_only"]["counterexamples"])
        == {"overwrote_intervening_edit", "used_obsolete_delegation"},
        "version_only_still_detects_delegation_fault":
        "used_obsolete_delegation" in orders["version_only"]["counterexamples"],
        "combined_commit_has_no_observed_fault":
        orders["version_and_authority"]["invalid_schedules"] == 0,
        "combined_commit_permits_some_work": orders["version_and_authority"]["commits"] > 0,
        "split_receipt_detects_duplicate": crashes["separate_effect_receipt"]["invalid_cuts"] > 0,
        "atomic_receipt_has_no_observed_duplicate": crashes["atomic_effect_receipt"]["invalid_cuts"] == 0,
    }
    print(json.dumps({"model_version": 1, "environment": "host_finite_model",
                      "concurrency": orders, "recovery": crashes, "checks": checks,
                      "passed": all(checks.values())}, indent=2, sort_keys=True))
    return 0 if all(checks.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
