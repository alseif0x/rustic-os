# SPDX-License-Identifier: Apache-2.0
"""Negative evidence tests for real scheduling, current authority and non-replay."""
import copy
import unittest
from tools.contracts.catalog import Catalog
from tools.contracts.scheduling_evidence import check_scheduling
from tools.contracts.validation import ContractError
from tools.contracts.tests.scheduling_fixtures import scheduling_cases


class SchedulingEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.catalog = Catalog()

    def test_scheduling_has_a_queued_observation_and_matching_terminal_receipt(self):
        self.assertEqual(check_scheduling(self.catalog, scheduling_cases()), 4)

    def test_missing_duplicate_or_unverified_cases_do_not_establish_scheduling(self):
        for mutate in (lambda c: c.pop(), lambda c: c.__setitem__(1, copy.deepcopy(c[0])),
                       lambda c: c[0].__setitem__("verified", False),
                       lambda c: c[0].__setitem__("reboot_verified", False)):
            cases = scheduling_cases(); mutate(cases)
            with self.assertRaises(ContractError): check_scheduling(self.catalog, cases)

    def test_preparation_io_flags_and_unrelated_receipts_cannot_invent_execution(self):
        for mutate in (lambda c: c[0]["observations"][0].__setitem__("phase", "running"),
                       lambda c: c[0]["observations"][0].__setitem__("pending", 1),
                       lambda c: c[0]["observations"][0].__setitem__("pending", False),
                       lambda c: c[0]["observations"][3].__setitem__("requested", "true"),
                       lambda c: c[0].pop("completion"),
                       lambda c: c[0]["completion"].__setitem__("operation_id", "another_operation"),
                       lambda c: c[0]["completion"]["receipt"].__setitem__("sha256", "0" * 64),
                       lambda c: c[0]["durable"][1].__setitem__("id", c[0]["durable"][0]["id"])):
            cases = scheduling_cases(); mutate(cases)
            with self.assertRaises(ContractError): check_scheduling(self.catalog, cases)

    def test_replay_revoked_execution_or_unproven_parity_are_rejected(self):
        for mutate in (lambda c: c[2]["recovered"][1].update(state="committed", terminal=6),
                       lambda c: c[2].__setitem__("fresh_explicit", False),
                       lambda c: c[3]["durable"][1].__setitem__("state", "committed"),
                       lambda c: c[3].__setitem__("revoked_before_execution", False),
                       lambda c: c[0]["peer_ack"].__setitem__("value", 1),
                       lambda c: c[1].__setitem__("stale_reply_rejected", False),
                       lambda c: c[0].__setitem__("retained_full", False)):
            cases = scheduling_cases(); mutate(cases)
            with self.assertRaises(ContractError): check_scheduling(self.catalog, cases)
