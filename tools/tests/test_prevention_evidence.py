# SPDX-License-Identifier: Apache-2.0
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import prevention_cases as evidence


class PreventionEvidence(unittest.TestCase):
    def state(self):
        return {"format": 5, "records": [
            {"admission": 10, "state": "cancelled", "terminal": 12,
             "prevention": "requested", "committed": 0}]}

    def test_cause_or_terminal_disagreement_cannot_pass_native_oracle_comparison(self):
        result = {"number": 10, "terminal": 12}
        for key, value in (("admission", 11), ("state", "admitted"), ("state", "committed"),
                           ("terminal", 13), ("prevention", "unknown"), ("committed", 12)):
            state = self.state()
            state["records"][0][key] = value
            with patch.object(evidence, "snapshot", return_value=(b"", state)):
                with self.assertRaises(AssertionError):
                    evidence.inspect(None, [result], ["requested"])

    def test_missing_duplicated_or_legacy_records_cannot_satisfy_migration(self):
        for mutation in (lambda s: s.update(format=4), lambda s: s.update(records=[]),
                         lambda s: s["records"].append(dict(s["records"][0]))):
            state = self.state()
            mutation(state)
            with patch.object(evidence, "snapshot", return_value=(b"", state)):
                with self.assertRaises(AssertionError):
                    evidence.inspect(None, [{"number": 10, "terminal": 12}], ["requested"])

    def test_checkpoint_cannot_hide_a_changed_cause(self):
        state = self.state()
        state["records"][0]["prevention"] = "authority_lost"
        report = {"results": [{"number": 10, "terminal": 12}], "reasons": ["requested"],
                  "selected_sha256": "original"}
        with patch.object(evidence, "snapshot", return_value=(b"", state)):
            with self.assertRaises(AssertionError):
                evidence.checkpoint(None, report)
        self.assertEqual(report["selected_sha256"], "original")
