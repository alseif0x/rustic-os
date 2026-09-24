# SPDX-License-Identifier: Apache-2.0
"""V7 maintenance output must decode to one report or one refusal and agree with the oracle views."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_retention import check_maintained, decode_hold, decode_maintain, reclaimable


REPORT = ("maintain-v7\r\nmaintain-v7 previous=e_0000000000000001 epoch=e_0000000000000002 records=8 "
          "sectors=56 job=4 ticks=31\r\nrustic:/workspaces> ")


def record(slot, aliases_live, runs):
    return {"slot": slot, "aliases_live": aliases_live, "runs": runs}


def snapshot(epoch, sequence, records, free, files=("same",)):
    return {"epoch": epoch, "sequence": sequence, "records": records, "free_sectors": free, "files": list(files)}


class Maintain(unittest.TestCase):
    def test_the_report_names_both_epochs_and_what_was_reclaimed(self):
        self.assertEqual(decode_maintain(REPORT), {"previous": 1, "epoch": 2, "records": 8, "sectors": 56,
                                                   "job": 4, "ticks": 31})
        self.assertEqual(decode_maintain("maintain-v7\r\nerror: Busy\r\n> "), {"error": "Busy"})

    def test_an_epoch_that_does_not_advance_by_one_or_ambiguous_output_is_rejected(self):
        for text in (REPORT.replace("e_0000000000000002", "e_0000000000000003"),
                     REPORT.replace("rustic:", "error: Busy\r\nrustic:"),
                     REPORT.replace(" ticks=31", ""),
                     "maintain-v7\r\n> "):
            with self.assertRaises(ValueError):
                decode_maintain(text)


class Hold(unittest.TestCase):
    def test_the_hold_line_reports_the_maintenance_and_the_abort(self):
        text = "replace-pattern-v7 a b c d e 1 4096 hold 40\r\nhold-v7 chunks=40 bytes=1600 maintain=Busy abort=ok\r\n> "
        self.assertEqual(decode_hold(text), {"chunks": 40, "bytes": 1600, "maintain": "Busy", "abort": "ok"})
        self.assertEqual(decode_hold("error: Full\r\n"), {"error": "Full"})

    def test_a_maintenance_report_or_a_receipt_inside_a_hold_is_ambiguous(self):
        for text in ("hold-v7 chunks=40 bytes=1600 maintain=Busy abort=ok\r\n" + REPORT,
                     "hold-v7 chunks=40 bytes=1600 maintain=Busy\r\n",
                     "write-v7 size=4096 ticks=3\r\n"):
            with self.assertRaises(ValueError):
                decode_hold(text)


class Oracle(unittest.TestCase):
    def setUp(self):
        self.before = snapshot(1, 20, [record(0, True, [(0, 8)]), record(1, False, [(8, 8)]),
                                       record(2, False, [(16, 4), (40, 4)])], 100)
        self.report = {"previous": 1, "epoch": 2, "records": 3, "sectors": 16}

    def test_only_snapshot_only_sectors_are_reclaimable(self):
        self.assertEqual(reclaimable(self.before), 16)

    def test_the_report_must_match_both_views(self):
        after = snapshot(2, 21, [], 116)
        check_maintained(self.before, after, self.report)
        for bad in (snapshot(2, 21, [], 115), snapshot(2, 22, [], 116), snapshot(3, 21, [], 116),
                    snapshot(2, 21, [record(0, True, [(0, 8)])], 116), snapshot(2, 21, [], 116, ("changed",))):
            with self.subTest(after=bad), self.assertRaises(AssertionError):
                check_maintained(self.before, bad, self.report)
        for field, value in (("records", 2), ("sectors", 24), ("previous", 2)):
            with self.subTest(field=field), self.assertRaises(AssertionError):
                check_maintained(self.before, after, dict(self.report, **{field: value}))


if __name__ == "__main__":
    unittest.main()
