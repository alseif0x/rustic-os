# SPDX-License-Identifier: Apache-2.0
"""V7 tracked-write output must decode to one receipt or one refusal and match the oracle."""
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_write import (LOOKUP_TIMING, SHELL_SUBJECT, check_absent, check_receipt, decode,
                                       decode_cut, lookup_error, match_records, pattern)


LINEAGE = "ab" * 16
WORKSPACE = f"ws_{LINEAGE}_00000005"
RESOURCE = f"rs_{LINEAGE}_00000005_00000008"


def answer(size=513, previous=7, version=8, instance=8, key=0x100, seed=1, ticks=12):
    digest = hashlib.sha256(pattern(seed, size)).hexdigest()
    return (f"replace-pattern-v7 {WORKSPACE} {RESOURCE} v_{previous:016x} e_{1:016x} k_{key:016x} {seed} {size}\r\n"
            f"operation-v1 id=op_{LINEAGE}_{version:016x} service_instance=si_{LINEAGE}_{instance:016x} "
            "state=succeeded effect=committed cancel_requested=false\r\n"
            f"receipt workspace={WORKSPACE} resource={RESOURCE} previous_version=v_{previous:016x} "
            f"version=v_{version:016x} size={size} epoch=e_{1:016x} key=k_{key:016x} sha256={digest}\r\n"
            f"write-v7 size={size} ticks={ticks}\r\nrustic:/workspaces> ")


class Pattern(unittest.TestCase):
    def test_matches_the_shell_formula_and_distinct_seeds_differ_everywhere(self):
        self.assertEqual(pattern(0, 4), bytes([0, 7, 14, 21]))
        self.assertEqual(pattern(1, 1), bytes([31]))
        self.assertEqual(pattern(0, 510)[509], (7 * 509 + 1) & 0xFF)
        left, right = pattern(3, 4096), pattern(103, 4096)
        self.assertTrue(all(a != b for a, b in zip(left, right)))


class Decode(unittest.TestCase):
    def test_receipt_fields_timing_and_exact_lines(self):
        result = decode(answer())
        self.assertEqual((result["previous"], result["version"], result["size"], result["key"]), (7, 8, 513, 0x100))
        self.assertEqual(result["ticks"], 12)
        self.assertEqual(len(result["lines"]), 2)
        self.assertTrue(result["lines"][1].startswith("receipt workspace="))

    def test_one_error_is_a_refusal(self):
        self.assertEqual(decode("replace-pattern-v7 x\r\nerror: Full\r\n> "), {"error": "Full"})
        self.assertEqual(decode("error: IdempotencyConflict\r\n"), {"error": "IdempotencyConflict"})

    def test_ambiguous_or_incomplete_output_is_rejected(self):
        for text in (
            answer().replace("write-v7 size=513 ticks=12\r\n", ""),
            answer() + answer(),
            answer().replace("rustic:", "error: Full\r\nrustic:"),
            "error: Full\r\nerror: Version\r\n",
            answer().replace("write-v7 size=513", "write-v7 size=514"),
        ):
            with self.assertRaises(ValueError):
                decode(text)


class Lookup(unittest.TestCase):
    def lookup(self, ticks=30):
        text = answer().split("\r\n", 1)[1].replace("write-v7 size=513 ticks=12", f"lookup-v7 size=513 ticks={ticks}")
        return f"operation-v7 op_{LINEAGE}_{8:016x}\r\n" + text

    def test_a_lookup_prints_the_same_receipt_lines_as_the_commit(self):
        found = decode(self.lookup(), LOOKUP_TIMING)
        self.assertEqual(found["lines"], decode(answer())["lines"])
        self.assertEqual(found["ticks"], 30)

    def test_a_write_answer_is_not_a_lookup_and_refusals_are_single(self):
        with self.assertRaises(ValueError):
            decode(answer(), LOOKUP_TIMING)
        self.assertEqual(lookup_error("operation-v7 x\r\nerror: OutcomeUnknown\r\n> "), "OutcomeUnknown")
        for text in (self.lookup(), "error: Denied\r\nerror: Denied\r\n"):
            with self.assertRaises(ValueError):
                lookup_error(text)


class Cut(unittest.TestCase):
    def test_the_cut_line_reports_both_observations(self):
        text = "replace-pattern-v7 a b c d e 6 524288 cut 400\r\ncut-v7 chunks=400 bytes=16000 job=3 old=Closed new=NoTransfer ticks=2\r\n> "
        self.assertEqual(decode_cut(text), {"chunks": 400, "bytes": 16000, "job": 3, "old": "Closed", "new": "NoTransfer",
                                            "ticks": 2})
        self.assertEqual(decode_cut("error: Version\r\n"), {"error": "Version"})

    def test_a_committed_or_ambiguous_cut_is_rejected(self):
        for text in (answer(), "cut-v7 chunks=400 bytes=16000 job=3 old=Closed new=NoTransfer ticks=2\r\nerror: Full\r\n",
                     "cut-v7 chunks=400 bytes=16000 job=3 old=Closed ticks=2\r\n",
                     "cut-v7 chunks=400 bytes=16000 job=3 old=Closed new=NoTransfer\r\n"):
            with self.assertRaises(ValueError):
                decode_cut(text)

    def test_the_revoked_key_must_leave_no_shell_record(self):
        seed_record = {"subject": 1, "key": 0x180}
        check_absent({"records": [seed_record]}, 0x180)
        with self.assertRaises(AssertionError):
            check_absent({"records": [seed_record, {"subject": SHELL_SUBJECT, "key": 0x180}]}, 0x180)


class Checks(unittest.TestCase):
    def test_receipt_must_match_request_and_content(self):
        receipt = decode(answer())
        check_receipt(receipt, LINEAGE, WORKSPACE, RESOURCE, 7, 1, 0x100, pattern(1, 513))
        with self.assertRaises(AssertionError):
            check_receipt(receipt, LINEAGE, WORKSPACE, RESOURCE, 7, 1, 0x100, pattern(2, 513))
        with self.assertRaises(AssertionError):
            check_receipt(receipt, LINEAGE, WORKSPACE, RESOURCE, 6, 1, 0x100, pattern(1, 513))

    def test_every_receipt_needs_one_matching_oracle_record(self):
        receipt = decode(answer())
        record = {"slot": 2, "subject": SHELL_SUBJECT, "state": "direct_committed", "workspace": 5, "object": 8,
                  "epoch": 1, "key": 0x100, "previous": 7, "committed": 8, "terminal": 8, "length": 513,
                  "sha256": receipt["sha256"], "instance": 8}
        seed_record = dict(record, subject=1, key=6, slot=0)
        self.assertEqual(match_records({"records": [seed_record, record]}, [receipt], 5, 8),
                         [{"slot": 2, "key": 0x100, "committed": 8}])
        for field, value in (("sha256", "0" * 64), ("instance", 9), ("length", 512), ("subject", 1)):
            with self.subTest(field=field), self.assertRaises(AssertionError):
                match_records({"records": [dict(record, **{field: value})]}, [receipt], 5, 8)


if __name__ == "__main__":
    unittest.main()
