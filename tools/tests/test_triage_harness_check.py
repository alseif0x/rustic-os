# SPDX-License-Identifier: Apache-2.0
"""Offline native-control consumer: source association and no provider access."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools.research.failure_triage.check_harness import check
from tools.research.failure_triage.format import RequestError


class HarnessCheckTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for mode, outcome in (("panic", "panic"), ("hang", "timeout")):
            directory = self.root / "boot" / mode
            directory.mkdir(parents=True)
            result = {"outcome": outcome, "returncode": 35 if mode == "panic" else -9,
                      "timed_out": mode == "hang", "build_id": "abc", "image_sha256": "a" * 64,
                      "elapsed_seconds": 1, "timeout_seconds": 1, "memory_mib": 256}
            raw = (json.dumps(result) + "\n").encode()
            serial = f"RUSTIC {mode.upper()} deliberate=1\n".encode()
            (directory / "result.json").write_bytes(raw)
            (directory / "serial.log").write_bytes(serial)
            harness = {"schema_version": 1, "producer": "rustic-boot-suite/v1",
                       "suite_run_id": "00000000-0000-4000-8000-000000000001", "mode": mode, "expected_outcome": outcome,
                       "observed_outcome": outcome, "reached_fixture": True, "harness_passed": True,
                       "build_id": "abc", "image_sha256": "a" * 64,
                       "result_sha256": hashlib.sha256(raw).hexdigest(),
                       "serial_sha256": hashlib.sha256(serial).hexdigest()}
            (directory / "harness.json").write_text(json.dumps(harness))

    def test_passed_controls_keep_outcomes_without_provider_access(self):
        with mock.patch("tools.research.failure_triage.diagnose.api_key", side_effect=AssertionError("credentials")), \
             mock.patch("tools.research.failure_triage.diagnose.post_json", side_effect=AssertionError("network")):
            summary = check(self.root / "boot", self.root / "out")
        self.assertEqual(summary["live_calls"], 0)
        self.assertEqual(len(summary["controls"]), 2)
        for row in summary["controls"]:
            self.assertEqual(row["guard"], "harness_passed")
            self.assertFalse(row["hypothesis"]["accepted"])

    def test_mixed_suite_controls_are_refused(self):
        path = self.root / "boot" / "hang" / "harness.json"
        value = json.loads(path.read_text())
        value["suite_run_id"] = "00000000-0000-4000-8000-000000000002"
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(RequestError, "different suite runs"):
            check(self.root / "boot", self.root / "out")

    def test_serial_replacement_is_refused(self):
        (self.root / "boot" / "panic" / "serial.log").write_text("different run\n")
        with self.assertRaisesRegex(RequestError, "serial does not match"):
            check(self.root / "boot", self.root / "out")


if __name__ == "__main__":
    unittest.main()
