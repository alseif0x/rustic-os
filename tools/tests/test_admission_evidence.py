# SPDX-License-Identifier: Apache-2.0
from pathlib import Path
import struct
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.oracle_admission import decode, numbers


class AdmissionOracle(unittest.TestCase):
    def test_v5_causes_are_versioned_and_exclusive_to_prevention(self):
        for reason, name in enumerate(("unknown", "requested", "version_conflict", "authority_lost")):
            p, record = self.record(2)
            p[81] = reason
            decode(p, 5, record, 12)
            self.assertEqual(record["prevention"], name)
            if reason:
                with self.assertRaises(AssertionError):
                    decode(p, 4, dict(record), 12)
        for state in (1, 3):
            p, record = self.record(state)
            decode(p, 5, record, 12)
            self.assertIsNone(record["prevention"])
            p[81] = 1
            with self.assertRaises(AssertionError):
                decode(p, 5, record, 12)

    def test_v5_rejects_unknown_codes_and_reserved_bytes(self):
        for offset, value in ((81, 4), (81, 255), (82, 1), (511, 1)):
            p, record = self.record(2)
            p[offset] = value
            with self.assertRaises(AssertionError):
                decode(p, 5, record, 12)

    def record(self, state):
        p = bytearray(1536)
        terminal = 0 if state == 1 else 12
        struct.pack_into("<QQB", p, 64, 10, terminal, state)
        record = {"previous": 4, "instance": 10, "committed": 12 if state == 3 else 0}
        return p, record

    def test_only_consistent_admission_states_are_accepted(self):
        for state, name in [(1, "admitted"), (2, "cancelled"), (3, "committed")]:
            p, record = self.record(state)
            decode(p, 4, record, 12)
            self.assertEqual(record["state"], name)
            self.assertEqual(numbers(record), {10} if state == 1 else {10, 12})
            for version in (2, 3):
                with self.assertRaises(AssertionError):
                    decode(p, version, dict(record), 12)
            for offset, value in [(64, 0), (64, 13), (80, 0), (80, 4), (81, 1), (511, 1)]:
                bad = bytearray(p)
                bad[offset] = value
                with self.assertRaises(AssertionError):
                    decode(bad, 4, dict(record), 12)

    def test_terminal_result_cannot_be_invented_from_an_admission(self):
        for state in (1, 2, 3):
            p, record = self.record(state)
            for field, value in [("previous", 10), ("instance", 0), ("instance", 11), ("committed", 7)]:
                bad = {**record, field: value}
                with self.assertRaises(AssertionError):
                    decode(p, 4, bad, 12)
            for terminal in (0, 9, 10, 13):
                if state == 1 and terminal == 0:
                    continue
                bad = bytearray(p)
                struct.pack_into("<Q", bad, 72, terminal)
                with self.assertRaises(AssertionError):
                    decode(bad, 4, dict(record), 12)
