# SPDX-License-Identifier: Apache-2.0
"""Challenge text evidence parsing; native execution remains a separate requirement."""
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.activity_cases import activity


class ActivityEvidenceTests(unittest.TestCase):
    def test_complete_kernel_diagnostic_can_interleave_a_queued_acknowledgement(self):
        line = "admission-activity-v1 id=ad_" + "07" * 16 + "_0000000000000009 service_instance=si_" + "07" * 16 + "_0000000000000008 phase=queued cancel_requested=0 io_pending=0"
        diagnostic = "RUSTIC IO_OBSERVATION held=1 owner=3 request=392\r\n"
        self.assertEqual(activity(line[:35] + diagnostic + line[35:]), activity(line))
        for bad in (diagnostic.rstrip(), diagnostic.replace("392", "bad"), "error: Uncertain\r\n"):
            with self.subTest(bad=bad), self.assertRaises(AssertionError):
                activity(line[:35] + bad + line[35:])

    def test_rejects_missing_ambiguous_or_terminal_live_observations(self):
        line = "admission-activity-v1 id=ad_" + "07" * 16 + "_0000000000000009 service_instance=si_" + "07" * 16 + "_0000000000000008 phase=stopping cancel_requested=1 io_pending=1"
        self.assertEqual(activity(line)["phase"], "stopping")
        for bad in ("", line + "\n" + line, line + "\nerror: Uncertain",
                    line.replace("stopping", "cancelled"), line.replace("io_pending=1", "io_pending=yes")):
            with self.subTest(bad=bad), self.assertRaises(AssertionError):
                activity(bad)
