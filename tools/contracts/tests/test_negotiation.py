# SPDX-License-Identifier: Apache-2.0
"""Reject a bundled Rust digest that drifts from the actual expanded contract."""
from pathlib import Path
import re
import unittest
from tools.contracts.lifecycle_conformance import catalog


class Negotiation(unittest.TestCase):
    def test_guest_bundle_matches_reviewed_expanded_descriptors(self):
        root = Path(__file__).resolve().parents[3]
        source = (root / 'crates/abi/src/files/negotiation/reviewed.rs').read_text()
        c = catalog()
        for symbol, method in [('GET', 'operations.get'), ('CANCEL', 'operations.cancel')]:
            body = re.search(rf'const {symbol}: \[u8; 32\] = \[(.*?)\];', source, re.S)[1]
            digest = bytes(int(value, 16) for value in re.findall(r'0x([0-9a-f]{2})', body))
            self.assertEqual(len(digest), 32)
            self.assertEqual(digest.hex(), c.digest(method))
