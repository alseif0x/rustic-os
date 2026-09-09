# SPDX-License-Identifier: Apache-2.0
"""Contract tests: success text alone must never create a passing boot."""
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.runner import classify


class OutcomeTests(unittest.TestCase):
    def test_success_needs_matching_build_and_exit_status(self):
        line = "RUSTIC SUCCESS component=boot build=abc"
        self.assertEqual(classify(33, False, line, "abc"), "success")
        self.assertEqual(classify(0, False, line, "abc"), "unexpected")
        self.assertEqual(classify(33, False, line, "other"), "unexpected")
        self.assertEqual(classify(33, False, line + "def", "abc"), "unexpected")

    def test_timeout_and_fatal_cannot_be_hidden_by_success_text(self):
        line = "RUSTIC SUCCESS component=boot build=abc"
        self.assertEqual(classify(33, True, line, "abc"), "timeout")
        self.assertEqual(classify(33, False, line + "\nRUSTIC PANIC", "abc"), "unexpected")
        self.assertEqual(classify(37, False, "RUSTIC FATAL", "abc"), "fatal")

    def test_crash_or_firmware_exit_is_not_a_guest_panic(self):
        self.assertEqual(classify(-11, False, "RUSTIC START", "abc"), "unexpected")
        self.assertEqual(classify(35, False, "", "abc"), "unexpected")
        self.assertEqual(classify(35, False, "RUSTIC PANIC", "abc"), "panic")
