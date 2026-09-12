# SPDX-License-Identifier: Apache-2.0
"""Malformed or unrelated probe failures cannot establish capability support."""
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.discovery_cases import operation_support


class DiscoveryProbeTests(unittest.TestCase):
    def test_requires_the_exact_unused_key_outcomes(self):
        self.assertFalse(operation_support("error: Unsupported\r\n"))
        for error in ("OutcomeUnknown", "ExpiredEpoch"):
            self.assertTrue(operation_support("error: " + error))
        for invalid in ("", "error: Busy", "error: Denied", "error: Uncertain",
                        "error: Unsupported\nerror: OutcomeUnknown", "error: Protocol"):
            with self.subTest(invalid=invalid), self.assertRaises(AssertionError):
                operation_support(invalid)
