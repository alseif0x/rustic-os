# SPDX-License-Identifier: Apache-2.0
"""Contract tests: success text alone must never create a passing boot."""
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.image import MEMORY_PROFILES, OUTPUT, image_directory, memory_supported
from boot_support.runner import classify, guest_memory


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

    def test_the_declared_profiles_are_the_ones_the_runner_accepts(self):
        self.assertEqual(MEMORY_PROFILES, (256, 512, 2048))
        for value in MEMORY_PROFILES:
            self.assertEqual(guest_memory({"memory_mib": value}), value)
        # An image built before the profile flag keeps the reference machine.
        self.assertEqual(guest_memory({}), 256)

    def test_a_non_reference_profile_never_overwrites_the_reference_image(self):
        self.assertEqual(image_directory("ok"), OUTPUT / "ok")
        for value in (512, 2048):
            self.assertEqual(image_directory("ok", value), OUTPUT / f"ok-{value}")
            self.assertNotEqual(image_directory("ok", value), image_directory("ok"))

    def test_a_pinned_harness_refuses_a_profile_it_cannot_boot(self):
        # These harnesses own QEMU and hardcode the reference size, so the image
        # builder refuses any other profile for them instead of recording it.
        for mode in ("terminal-test", "recovery-test"):
            self.assertTrue(memory_supported(mode, 256))
            for value in (512, 2048):
                self.assertFalse(memory_supported(mode, value))
        for mode in ("ok", "block-user"):
            for value in MEMORY_PROFILES:
                self.assertTrue(memory_supported(mode, value))
        self.assertFalse(memory_supported("ok", 1024))
