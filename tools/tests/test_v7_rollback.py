# SPDX-License-Identifier: Apache-2.0
"""Rollback evidence must name the older pinned pair and an unchanged volume."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_rollback import check_order, check_selection, pin, unchanged


LINEAGE = "44" * 16
WORKSPACE = f"ws_{LINEAGE}_00000005"


def answer(object_id, version):
    return {"workspace": {"id": 5, "text": WORKSPACE},
            "file": {"id": object_id, "version": version, "resource": f"rs_{LINEAGE}_00000005_{object_id:08x}"}}


def published():
    return {1: {"elf": answer(8, 14), "manifest": answer(9, 16)},
            2: {"elf": answer(10, 18), "manifest": answer(11, 20)}}


def finish(tag, elf, manifest):
    return {"report": {"report": 7, "bytes": tag}, "staged_versions": {"elf": elf, "manifest": manifest}}


class Pins(unittest.TestCase):
    def test_a_pin_names_the_workspace_both_resources_and_versions(self):
        self.assertEqual(pin(published()[1]), (WORKSPACE, f"rs_{LINEAGE}_00000005_00000008",
                                               f"rs_{LINEAGE}_00000005_00000009",
                                               "v_000000000000000e", "v_0000000000000010"))

    def test_tag_one_must_be_published_before_tag_two(self):
        check_order(published())
        swapped = published()
        swapped[1], swapped[2] = swapped[2], swapped[1]
        with self.assertRaisesRegex(AssertionError, "older"):
            check_order(swapped)
        interleaved = published()
        interleaved[1]["manifest"] = answer(9, 19)
        with self.assertRaisesRegex(AssertionError, "older"):
            check_order(interleaved)


class Selection(unittest.TestCase):
    def setUp(self):
        self.pins = {tag: pin(pair) for tag, pair in published().items()}
        self.newer = finish(2, "v_0000000000000012", "v_0000000000000014")
        self.older = finish(1, "v_000000000000000e", "v_0000000000000010")

    def test_newer_then_older_with_their_own_tags_and_pins(self):
        check_selection([(2, self.newer), (1, self.older)], self.pins)

    def test_a_wrong_tag_pin_or_order_is_refused(self):
        wrong_tag = finish(2, "v_000000000000000e", "v_0000000000000010")
        stale_pin = finish(1, "v_0000000000000012", "v_0000000000000010")
        for finishes in ([(2, self.newer), (1, wrong_tag)], [(2, self.newer), (1, stale_pin)],
                         [(1, self.older), (2, self.newer)], [(2, self.newer)]):
            with self.assertRaises(AssertionError, msg=finishes):
                check_selection(finishes, self.pins)


class Unchanged(unittest.TestCase):
    def snapshot(self):
        return {"lineage": LINEAGE, "generation": 1, "recovered": False, "sequence": 20, "epoch": 1, "next": 12,
                "nodes": {5: {"version": 8}}, "files": [{"id": 8}], "contents": {"/a": b"x"},
                "records": [{"slot": 0}], "free_sectors": 9, "rejected_headers": {}}

    def test_identical_snapshots_differ_nowhere(self):
        self.assertEqual(unchanged(self.snapshot(), self.snapshot()), [])

    def test_each_changed_field_is_named(self):
        for key, value in (("sequence", 21), ("generation", 0), ("records", []), ("contents", {"/a": b"y"}),
                           ("free_sectors", 8), ("recovered", True)):
            after = dict(self.snapshot(), **{key: value})
            self.assertEqual(unchanged(self.snapshot(), after), [key])


if __name__ == "__main__":
    unittest.main()
