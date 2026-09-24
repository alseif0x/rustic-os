# SPDX-License-Identifier: Apache-2.0
"""The V7 mid-publication revocation diagnostic must decode to one revocation line and one outcome."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_authority import check_revoked, decode_revoke


LINEAGE = "33" * 16
CANCELLED = (f"execute-admission-v7 x revoke 0 200\r\nrevoke-v7 held=1 job=2 old=Uncertain ticks=205\r\n"
             f"admission-v1 id=ad_{LINEAGE}_0000000000000008 service_instance=si_{LINEAGE}_0000000000000008 "
             "state=cancelled terminal=9\r\nrustic> ")
MISSING = "admit-pattern-v7 x\r\nrevoke-v7 held=1 job=3 old=Uncertain ticks=200\r\nerror: OutcomeUnknown\r\n> "


class Revoke(unittest.TestCase):
    def test_the_revocation_line_and_the_outcome_on_the_new_binding_decode(self):
        revoked = decode_revoke(CANCELLED)
        self.assertEqual((revoked["held"], revoked["job"], revoked["old"], revoked["ticks"]),
                         (True, 2, "Uncertain", 205))
        self.assertEqual((revoked["outcome"]["state"], revoked["outcome"]["terminal"]), ("cancelled", 9))
        check_revoked(revoked, "execution")
        self.assertEqual(decode_revoke(MISSING)["outcome"], {"error": "OutcomeUnknown"})

    def test_a_missing_or_duplicated_revocation_line_is_rejected(self):
        with self.assertRaises(ValueError):
            decode_revoke("execute-admission-v7 x\r\nerror: Busy\r\n> ")
        line = "revoke-v7 held=1 job=2 old=Uncertain ticks=5\r\n"
        with self.assertRaises(ValueError):
            decode_revoke(line + MISSING)
        with self.assertRaises(ValueError):
            decode_revoke(MISSING.replace("job=3", "job=0"))

    def test_an_unheld_publication_or_a_delivered_reply_fails_the_check(self):
        for text in (CANCELLED.replace("held=1", "held=0"), CANCELLED.replace("old=Uncertain", "old=reply")):
            with self.assertRaises(AssertionError):
                check_revoked(decode_revoke(text), "execution")


if __name__ == "__main__":
    unittest.main()
