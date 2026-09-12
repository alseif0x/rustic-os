# SPDX-License-Identifier: Apache-2.0
"""Challenge native discovery claims; a synthetic report never demonstrates a guest."""
import copy
import unittest
from tools.contracts.capabilities_conformance import (BOUNDS, METHODS, check_discovery, host_check,
                                                      items, native_check)
from tools.contracts.catalog import Catalog
from tools.contracts.validation import ContractError


def availability(operations=False):
    report = dict.fromkeys(METHODS, "unavailable")
    report["capabilities.list"] = "degraded"
    report["files.read"] = "available"
    if operations:
        report["files.replace"] = "available"
        report["operations.get"] = "available"
    return report


def evidence(operations=False):
    return {"verified": True, "kernel_sha256": "a" * 64, "read_contract": {"verified": True},
            "discovery": {"verified": True, "operations_enabled": operations,
                          "deterministic_client_agrees": True,
                          "availability": availability(operations), "bounds": dict(BOUNDS)}}


class CapabilityDiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.catalog = Catalog()

    def test_host_shape_accepts_the_catalog_and_rejects_invalid_reports(self):
        result = host_check(self.catalog)
        self.assertEqual((result["accepted_items"], result["rejected_vectors"]), (8, 4))
        self.assertEqual(result["unimplemented_methods"],
                         ["capabilities.list", "capabilities.describe"])

    def test_native_discovery_matches_the_mounted_volume(self):
        for operations in (False, True):
            with self.subTest(operations=operations):
                result = native_check(self.catalog, evidence(operations))
                self.assertEqual(result["items"], 8)
                self.assertEqual(result["availability"]["files.replace"],
                                 "available" if operations else "unavailable")

    def test_a_claim_that_contradicts_the_volume_is_rejected(self):
        bad = evidence(False)
        bad["discovery"]["availability"]["files.replace"] = "available"
        with self.assertRaises(ContractError):
            native_check(self.catalog, bad)
        bad = evidence(True)
        bad["discovery"]["operations_enabled"] = "yes"
        with self.assertRaises(ContractError):
            native_check(self.catalog, bad)

    def test_an_unexercised_method_cannot_be_advertised(self):
        for method in ("operations.cancel", "events.read", "system.status"):
            bad = evidence()
            bad["discovery"]["availability"][method] = "available"
            with self.subTest(method=method), self.assertRaises(ContractError):
                native_check(self.catalog, bad)
        missing_read = evidence()
        missing_read.pop("read_contract")
        with self.assertRaises(ContractError):
            native_check(self.catalog, missing_read)

    def test_registry_claims_bounds_and_identity_are_checked(self):
        for mutate in (lambda e: e["discovery"]["availability"].__setitem__("capabilities.list", "available"),
                       lambda e: e["discovery"]["availability"].__setitem__("capabilities.describe", "degraded"),
                       lambda e: e["discovery"]["bounds"].__setitem__("max_inline_bytes", 2048),
                       lambda e: e["discovery"].__setitem__("verified", False),
                       lambda e: e["discovery"].__setitem__("deterministic_client_agrees", False),
                       lambda e: e.__setitem__("kernel_sha256", "short")):
            bad = copy.deepcopy(evidence())
            mutate(bad)
            with self.assertRaises(ContractError):
                native_check(self.catalog, bad)

    def test_method_order_and_enumeration_are_part_of_the_contract(self):
        reordered = {method: "unavailable" for method in reversed(METHODS)}
        with self.assertRaises(ContractError):
            items(self.catalog, reordered)
        with self.assertRaises(ContractError):
            check_discovery(self.catalog, {"verified": True}, {})


if __name__ == "__main__":
    unittest.main()
