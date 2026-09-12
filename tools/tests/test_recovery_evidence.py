# SPDX-License-Identifier: Apache-2.0
"""Host evidence admission; these fixtures do not demonstrate guest execution."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.scenarios import reached
from terminal_support.operation_cases import operation


def transcript():
    return ("RusticOS native terminal 0.1\n" * 88
            + "error: Uncertain\n" * 17
            + "RUSTIC IO_OBSERVATION held=1\n" * 27
            + "admission-activity-v1\n" * 62 + "phase=queued\n"
            + "IdempotencyConflict\nExpiredEpoch\noperation-v1\npersistent format v3\npersistent format v4\nadmission-v1\n")


class RecoveryEvidenceTests(unittest.TestCase):
    def test_accepts_complete_recovery_inventory(self):
        self.assertTrue(reached("recovery-test", transcript()))

    def test_rejects_previous_inventory_and_duplicated_live_evidence(self):
        old = transcript().replace("RusticOS native terminal 0.1\n", "", 18)
        old = old.replace("RUSTIC IO_OBSERVATION held=1\n", "", 9)
        old = old.replace("admission-activity-v1\n", "", 34)
        old = old.replace("error: Uncertain\n", "", 2)
        self.assertFalse(reached("recovery-test", old))
        self.assertFalse(reached("recovery-test", transcript() + "admission-activity-v1\n"))

    def test_operation_parser_rejects_duplicate_or_ambiguous_results(self):
        valid = ("operation-v1 id=op_test service_instance=si_test state=succeeded effect=committed cancel_requested=false\n"
                 "receipt workspace=ws_test resource=rs_test previous_version=v_1 version=v_2 size=0 epoch=e_1 key=k_1 sha256=" + "0" * 64 + "\n")
        self.assertEqual(operation(valid)["receipt"]["size"], 0)
        for value in (valid.replace("state=succeeded", "state=failed state=succeeded"),
                      valid.replace("size=0", "size=00"), valid.replace("size=0", "size=1_024"),
                      valid.replace("effect=", "\teffect="), valid + "error: Uncertain\n",
                      valid.replace("\nreceipt", "\nintervening output\nreceipt"), valid * 2):
            with self.assertRaises(AssertionError):
                operation(value)

    def test_rejects_missing_cases_and_panics(self):
        for marker in ("RusticOS native terminal 0.1", "error: Uncertain",
                       "RUSTIC IO_OBSERVATION held=1", "IdempotencyConflict", "ExpiredEpoch",
                       "operation-v1", "persistent format v3", "persistent format v4", "admission-v1",
                       "admission-activity-v1", "phase=queued"):
            with self.subTest(missing=marker):
                self.assertFalse(reached("recovery-test", transcript().replace(marker, "", 1)))
        self.assertFalse(reached("recovery-test", transcript() + "RUSTIC PANIC"))


if __name__ == "__main__":
    unittest.main()
