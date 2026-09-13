# SPDX-License-Identifier: Apache-2.0
"""Reject missing, contradictory, leaked or request-triggered lifecycle evidence."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.tasks_lifecycle import parse, PREFIX, FLAGS


def evidence():
    facts = {
        "cancel": "job=10 task_pid=9 slot=0",
        "concurrent": "rows=2 task_pid=12 spin_pid=13 job=11",
        "expiry": "rows=2 start_tick=100 complete_tick=110 expiry_tick=1110 query_sent_tick=1410",
    }
    counts = "before_frames=50000 after_frames=50000 before_processes=3 after_processes=3 before_channels=4 after_channels=4 before_pending=0 after_pending=0"
    return "\n".join(PREFIX + f"case={case} " + " ".join(f"{flag}=1" for flag in FLAGS[case].split())
                     + " " + facts[case] + " " + counts for case in FLAGS)


class TasksLifecycleTests(unittest.TestCase):
    def test_complete_native_evidence(self):
        self.assertEqual(set(parse(evidence())), {"cancel", "concurrent", "expiry"})

    def test_missing_or_duplicate_cases(self):
        lines = evidence().splitlines()
        for value in ("\n".join(lines[:-1]), evidence() + "\n" + lines[0]):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                parse(value)

    def test_every_witness_is_required(self):
        for case, flags in FLAGS.items():
            for flag in flags.split():
                lines = evidence().splitlines()
                index = list(FLAGS).index(case)
                lines[index] = lines[index].replace(f"{flag}=1", f"{flag}=0")
                with self.subTest(case=case, flag=flag), self.assertRaises(AssertionError):
                    parse("\n".join(lines))

    def test_resource_leaks_and_invalid_identity(self):
        for before, after in (("after_frames=50000", "after_frames=49999"),
                              ("after_processes=3", "after_processes=4"),
                              ("after_channels=4", "after_channels=5"),
                              ("after_pending=0", "after_pending=1"),
                              ("spin_pid=13", "spin_pid=12"), ("slot=0", "slot=2"),
                              ("job=10", "job=0"), ("rows=2", "rows=1")):
            with self.subTest(after=after), self.assertRaises(AssertionError):
                parse(evidence().replace(before, after))

    def test_cleanup_must_precede_next_owner_request(self):
        for tick in ("1410", "1411", "110", "0"):
            with self.subTest(tick=tick), self.assertRaises(AssertionError):
                parse(evidence().replace("expiry_tick=1110", "expiry_tick=" + tick))

    def test_strict_schema_and_integer_encoding(self):
        for before, after in (("held=1", "held=1 held=1"), ("held=1", "held=01"),
                              ("held=1", "held=-1"), ("held=1", "held=١"),
                              ("held=1", "held"), ("held=1", "invented=1")):
            with self.subTest(after=after), self.assertRaises(AssertionError):
                parse(evidence().replace(before, after, 1))
