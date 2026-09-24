# SPDX-License-Identifier: Apache-2.0
"""V7 admission output must decode to one consistent status or one refusal and agree with the oracle record."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_admission import check_record, decode_observation, decode_status


LINEAGE = "22" * 16
ADMITTED = (f"admission x\r\nadmission-v1 id=ad_{LINEAGE}_0000000000000008 "
            f"service_instance=si_{LINEAGE}_0000000000000008 state=admitted terminal=0\r\nrustic> ")
COMMITTED = (f"admission-v1 id=ad_{LINEAGE}_0000000000000008 service_instance=si_{LINEAGE}_0000000000000008 "
             f"state=committed terminal=9\r\ncompletion=op_{LINEAGE}_0000000000000009\r\n> ")
OBSERVED = (f"admission-observation-v2 profile=2 id=ad_{LINEAGE}_000000000000000a "
            f"service_instance=si_{LINEAGE}_0000000000000009 kind=retained state=cancelled terminal=12 "
            "prevention=requested\r\n> ")


def record(**fields):
    base = {"subject": 2, "admission": 8, "state": "admitted", "terminal": 0, "instance": 8, "cause": None,
            "committed": 0, "workspace": 5, "object": 8, "epoch": 1, "key": 0x400, "previous": 7,
            "length": 3, "sha256": "ab", "runs": [(0, 1)], "slot": 2}
    return {**base, **fields}


EXPECTED = {"workspace": 5, "object": 8, "epoch": 1, "key": 0x400, "previous": 7, "length": 3, "sha256": "ab"}


class Status(unittest.TestCase):
    def test_admitted_and_committed_statuses_decode_with_their_completion(self):
        admitted = decode_status(ADMITTED)
        self.assertEqual((admitted["number"], admitted["state"], admitted["terminal"], admitted["completion"]),
                         (8, "admitted", 0, None))
        committed = decode_status(COMMITTED)
        self.assertEqual(committed["completion"], f"op_{LINEAGE}_0000000000000009")
        timed = decode_status(ADMITTED.replace("\r\nrustic", "\r\nadmit-v7 size=65536 ticks=12\r\nrustic"), True)
        self.assertEqual((timed["size"], timed["ticks"]), (65536, 12))
        self.assertEqual(decode_status("execute-admission x\r\nerror: Version\r\n> "), {"error": "Version"})

    def test_inconsistent_or_ambiguous_statuses_are_rejected(self):
        for text in (COMMITTED.replace("completion=op_" + LINEAGE + "_0000000000000009", "completion=op_x"),
                     COMMITTED.replace("\r\ncompletion", "\r\nnothing"),
                     ADMITTED.replace("terminal=0", "terminal=3"),
                     COMMITTED.replace("terminal=9", "terminal=8").replace("0009", "0008"),
                     ADMITTED.replace(f"si_{LINEAGE}", "si_" + "33" * 16),
                     ADMITTED.replace("rustic> ", "error: Busy\r\nrustic> "),
                     ADMITTED + ADMITTED):
            with self.assertRaises(ValueError):
                decode_status(text)
        with self.assertRaises(ValueError):
            decode_status(ADMITTED, timing=True)

    def test_a_cause_appears_exactly_for_a_cancelled_observation(self):
        self.assertEqual(decode_observation(OBSERVED)["prevention"], "requested")
        for text in (OBSERVED.replace("prevention=requested", "prevention=none"),
                     OBSERVED.replace("state=cancelled", "state=admitted"),
                     OBSERVED.replace("> ", "error: Busy\r\n> ")):
            with self.assertRaises(ValueError):
                decode_observation(text)


class Oracle(unittest.TestCase):
    def test_the_record_must_match_the_status_and_the_request(self):
        snapshot = {"lineage": LINEAGE, "records": [record()]}
        self.assertEqual(check_record(snapshot, decode_status(ADMITTED), EXPECTED)["state"], "admitted")
        committed = {"lineage": LINEAGE, "records": [record(state="admitted_committed", terminal=9, committed=9)]}
        check_record(committed, decode_status(COMMITTED), EXPECTED)

    def test_a_mismatched_or_missing_record_is_refused(self):
        status = decode_status(ADMITTED)
        for records in ([], [record(state="cancelled", cause="requested")], [record(subject=1)],
                        [record(sha256="cd")], [record(instance=7)], [record(), record(slot=3)]):
            with self.assertRaises(AssertionError):
                check_record({"lineage": LINEAGE, "records": records}, status, EXPECTED)


if __name__ == "__main__":
    unittest.main()
