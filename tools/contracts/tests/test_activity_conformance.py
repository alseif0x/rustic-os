# SPDX-License-Identifier: Apache-2.0
"""Challenge the declared live-control correspondence, especially what it must reject."""
import copy
import unittest
from tools.contracts.activity_conformance import (check_case, denial, host_check, live_operation,
                                                  native_check, record_operation, uncertain_operation)
from tools.contracts.catalog import Catalog
from tools.contracts.validation import ContractError

LINEAGE = "0" * 32
IDENTITY = {"id": "ad_" + LINEAGE + "_0000000000000003", "lineage": LINEAGE,
            "instance": "si_" + LINEAGE + "_0000000000000003"}


def observation(phase, requested=0, pending=1):
    return {**{k: IDENTITY[k] for k in ("id", "instance")}, "phase": phase,
            "requested": requested, "pending": pending}


def case(name, *, phases, state, terminal, committed, denied=0, uncertain=False):
    value = {"case": "public_activity_" + name, "verified": True, "denied": denied,
             "observations": [observation(*p) for p in phases],
             "durable": {**IDENTITY, "state": state, "terminal": terminal}}
    if uncertain:
        value["uncertain"] = True
        value["stopped"] = True
    else:
        value["committed"] = committed
    return value


def inventory():
    return [case("early", phases=(("running",), ("stopping", 1)), state="cancelled", terminal=4, committed=False),
            case("header", phases=(("running",), ("settling",)), state="committed", terminal=5, committed=True),
            case("flush", phases=(("running",), ("settling",)), state="committed", terminal=6, committed=True),
            case("inspect_only", phases=(("running",), ("running",)), state="committed", terminal=7,
                 committed=True, denied=17),
            case("foreign_scope", phases=(("running",), ("running",)), state="committed", terminal=8,
                 committed=True, denied=27),
            case("failed_drain", phases=(), state="admitted", terminal=0, committed=False, uncertain=True),
            case("saturated", phases=(("running",), ("stopping", 1), ("stopping", 1)),
                 state="cancelled", terminal=9, committed=False)]


def evidence(cases=None):
    return {"verified": True, "kernel_sha256": "a" * 64,
            "cases": [{"case": "unrelated", "verified": True}] + (cases or inventory())}


class ActivityCorrespondenceTests(unittest.TestCase):
    def setUp(self):
        self.catalog = Catalog()

    def test_host_correspondence_enumerates_accepted_and_rejected_vectors(self):
        result = host_check(self.catalog)
        self.assertEqual((result["accepted_vectors"], result["rejected_vectors"]), (8, 4))
        self.assertEqual(result["unimplemented_methods"], ["operations.cancel"])

    def test_native_inventory_maps_success_prevention_and_reconciliation(self):
        result = native_check(self.catalog, evidence())
        self.assertEqual(result["cases"], 7)
        self.assertEqual(result["states"]["public_activity_early"], "cancelled")
        self.assertEqual(result["states"]["public_activity_header"], "succeeded")
        self.assertEqual(result["states"]["public_activity_failed_drain"], "reconciling")

    def test_a_live_observation_is_never_a_terminal_result(self):
        for phase in ("cancelled", "committed", "succeeded"):
            with self.subTest(phase=phase), self.assertRaises(ContractError):
                live_operation(IDENTITY, observation(phase))
        running = live_operation({**IDENTITY, "state": "admitted", "terminal": 0}, observation("running", 1))
        self.assertEqual((running["state"], running["effect"]), ("running", "none"))
        self.assertTrue(running["cancel_requested"])

    def test_settling_never_claims_a_known_effect_or_a_rollback(self):
        settling = live_operation(IDENTITY, observation("settling"))
        self.assertEqual((settling["state"], settling["effect"]), ("reconciling", "unknown"))
        bad = copy.deepcopy(inventory())
        bad[1]["durable"]["state"] = "cancelled"
        bad[1]["durable"]["terminal"] = 0
        bad[1]["committed"] = False
        with self.assertRaises(ContractError):
            native_check(self.catalog, evidence(bad))

    def test_prevention_and_effects_must_match_the_record(self):
        for mutate in (lambda c: c[0].__setitem__("committed", True),
                       lambda c: c[0].__setitem__("observations", [observation("running")]),
                       lambda c: c[6]["observations"].__setitem__(2, observation("running"))):
            bad = copy.deepcopy(inventory())
            mutate(bad)
            with self.subTest(mutation=str(mutate)), self.assertRaises(ContractError):
                native_check(self.catalog, evidence(bad))

    def test_identity_changes_at_commit_and_is_checked(self):
        settled, committed = record_operation({**IDENTITY, "state": "committed", "terminal": 5}, True)
        self.assertTrue(committed)
        self.assertEqual(settled["operation_id"], "op_" + LINEAGE + "_0000000000000005")
        prevented, committed = record_operation({**IDENTITY, "state": "cancelled", "terminal": 4}, True)
        self.assertFalse(committed)
        self.assertEqual(prevented["operation_id"], IDENTITY["id"])
        for state, terminal in (("committed", 0), ("admitted", 4), ("cancelled", 0)):
            with self.subTest(state=state), self.assertRaises(ContractError):
                record_operation({**IDENTITY, "state": state, "terminal": terminal}, False)

    def test_uncertain_attempts_and_refusals_never_report_a_known_outcome(self):
        record = {**IDENTITY, "state": "admitted", "terminal": 0}
        result = uncertain_operation(record, True)
        self.assertEqual((result["state"], result["effect"]), ("reconciling", "unknown"))
        self.assertTrue(result["cancel_requested"])
        self.assertFalse(uncertain_operation(record)["cancel_requested"])
        self.assertEqual(denial(17)["code"], "access_denied")
        self.assertEqual(denial(27), {"code": "outcome_unknown", "next_action": "reconcile",
                                      "effect": "unknown"})
        with self.assertRaises(ContractError):
            denial(13)

    def test_incomplete_inventory_and_foreign_observations_are_rejected(self):
        with self.assertRaises(ContractError):
            native_check(self.catalog, evidence(inventory()[:-1]))
        with self.assertRaises(ContractError):
            native_check(self.catalog, {**evidence(), "kernel_sha256": "unknown"})
        bad = copy.deepcopy(inventory())
        bad[0]["observations"][0]["instance"] = "si_" + "1" * 32 + "_0000000000000003"
        with self.assertRaises(ContractError):
            native_check(self.catalog, evidence(bad))
        with self.assertRaises(ContractError):
            check_case(self.catalog, {"case": "public_activity_x", "verified": True})


if __name__ == "__main__":
    unittest.main()
