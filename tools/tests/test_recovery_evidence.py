# SPDX-License-Identifier: Apache-2.0
"""Host evidence admission; these fixtures do not demonstrate guest execution."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.scenarios import reached


def transcript():
    return ("RusticOS native terminal 0.1\n" * 18
            + "error: Uncertain\n" * 7
            + "RUSTIC IO_OBSERVATION held=1\n" * 2
            + "IdempotencyConflict\nExpiredEpoch\n")


class RecoveryEvidenceTests(unittest.TestCase):
    def test_accepts_complete_recovery_inventory(self):
        self.assertTrue(reached("recovery-test", transcript()))

    def test_rejects_missing_cases_and_panics(self):
        for marker in ("RusticOS native terminal 0.1", "error: Uncertain",
                       "RUSTIC IO_OBSERVATION held=1", "IdempotencyConflict", "ExpiredEpoch"):
            with self.subTest(missing=marker):
                self.assertFalse(reached("recovery-test", transcript().replace(marker, "", 1)))
        self.assertFalse(reached("recovery-test", transcript() + "RUSTIC PANIC"))


if __name__ == "__main__":
    unittest.main()
