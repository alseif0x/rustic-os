# SPDX-License-Identifier: Apache-2.0
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.runner import classify
from boot_support.scenarios import reached


class InterruptEvidence(unittest.TestCase):
    def test_exception_needs_its_exit_and_revision(self):
        line = "RUSTIC EXCEPTION build=abc vector=6 error=0x0 emergency=0"
        self.assertEqual(classify(39, False, line, "abc"), "exception")
        self.assertEqual(classify(39, False, line, "other"), "unexpected")
        self.assertEqual(classify(0, False, line, "abc"), "unexpected")
        self.assertEqual(classify(33, False, line + "\nRUSTIC SUCCESS component=boot build=abc", "abc"), "unexpected")

    def test_wrong_fault_or_stack_cannot_pass_double_fault_fixture(self):
        prefix = "RUSTIC FAULT_FIXTURE\nRUSTIC EXCEPTION build=abc "
        self.assertTrue(reached("doublefault", prefix + "vector=8 error=0x0 rip=0 emergency=1"))
        self.assertFalse(reached("doublefault", prefix + "vector=8 error=0x0 rip=0 emergency=0"))
        self.assertFalse(reached("doublefault", prefix + "vector=13 error=0xfff8 rip=0 emergency=1"))
        self.assertFalse(reached("gp", prefix + "vector=13 error=0x0 rip=0 emergency=0"))

    def test_boot_or_timer_failure_before_fixture_is_rejected(self):
        self.assertFalse(reached("timer-stall", "RUSTIC START"))
        self.assertTrue(reached("timer-stall", "RUSTIC TIMER_STALL reached_wait=1"))
        self.assertFalse(reached("ok", "RUSTIC SUCCESS component=boot build=abc"))
