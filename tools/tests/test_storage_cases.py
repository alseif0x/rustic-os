# SPDX-License-Identifier: Apache-2.0
"""Challenge the acceptance oracle; these fixtures are not native evidence."""
import sys
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.storage_cases import rejected


class StorageCasesTests(unittest.TestCase):
    def check_request(self, output, after=b"original"):
        uart = Mock()
        uart.command.return_value = output
        before_state = {"sequence": 42, "selected_sha256": "before"}
        after_state = {"sequence": 42, "selected_sha256": "after"}
        with patch("terminal_support.storage_cases.snapshot",
                   side_effect=[(b"original", before_state), (after, after_state)]):
            return rejected(uart, Path("unused"), "touch overflow", "Full")

    def test_rejects_a_failure_that_changed_storage(self):
        with self.assertRaisesRegex(AssertionError, "changed the selected disk"):
            self.check_request("touch overflow\r\nerror: Full\r\nrustic:/> ", b"modified")

    def test_rejects_missing_wrong_or_multiple_errors(self):
        for output in ("ok\n", "error: Invalid\n", "error: Full\nerror: Full\n",
                       "error: Full\nerror: Uncertain\n"):
            with self.subTest(output=output), self.assertRaisesRegex(AssertionError, "ambiguous"):
                self.check_request(output)

    def test_preserves_observed_denial_evidence(self):
        result = self.check_request("touch overflow\r\nerror: Full\r\nrustic:/> ")
        self.assertEqual(result["error"], "Full")
        self.assertEqual(result["sequence"], 42)


if __name__ == "__main__":
    unittest.main()
