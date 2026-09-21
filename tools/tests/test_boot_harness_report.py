# SPDX-License-Identifier: Apache-2.0
"""Focused host tests for boot-suite harness evidence."""

from __future__ import annotations

import json
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import uuid
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from tools.boot_support import harness_report
from tools.boot_support.block_runner import SECTORS
from tools.boot_support import runner


class BootHarnessReportTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    @staticmethod
    def result(outcome: str, *, timed_out: bool = False) -> dict[str, object]:
        return {
            "outcome": outcome,
            "returncode": 124 if timed_out else 33,
            "timed_out": timed_out,
            "elapsed_seconds": 1.25,
            "timeout_seconds": 3,
            "build_id": "build-1",
            "image_sha256": "a" * 64,
        }

    def write_mode(self, mode: str, result: dict[str, object], serial: str = "fixture\n") -> Path:
        directory = self.root / mode
        directory.mkdir(parents=True)
        (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        (directory / "serial.log").write_bytes(serial.encode())
        return directory

    def test_expected_hang_passes_only_after_fixture_reached(self):
        directory = self.write_mode(
            "hang", self.result("timeout", timed_out=True), "RUSTIC HANG deliberate=1\n"
        )
        suite_id = str(uuid.uuid4())
        passed = harness_report.write_report(
            directory / "harness.json", suite_id, "hang", "timeout",
            self.result("timeout", timed_out=True), True,
        )
        self.assertTrue(passed["harness_passed"])
        not_reached = harness_report.write_report(
            directory / "harness.json", suite_id, "hang", "timeout",
            self.result("timeout", timed_out=True), False,
        )
        self.assertFalse(not_reached["harness_passed"])
        self.assertEqual(not_reached["observed_outcome"], "timeout")

    def test_result_mismatch_and_missing_result_never_emit_acceptance(self):
        directory = self.write_mode("panic", self.result("panic"))
        with self.assertRaises(harness_report.HarnessReportError):
            harness_report.write_report(
                directory / "harness.json", str(uuid.uuid4()), "panic", "panic",
                self.result("success"), True,
            )
        self.assertFalse((directory / "harness.json").exists())
        (directory / "result.json").unlink()
        with self.assertRaises(harness_report.HarnessReportError):
            harness_report.write_report(
                directory / "harness.json", str(uuid.uuid4()), "panic", "panic",
                self.result("panic"), True,
            )
        self.assertFalse((directory / "harness.json").exists())

    def test_producer_hashes_large_native_serial_without_triage_size_caps(self):
        directory = self.write_mode("panic", self.result("panic"), "x" * (5 * 1024 * 1024))
        report = harness_report.write_report(
            directory / "harness.json", str(uuid.uuid4()), "panic", "panic",
            self.result("panic"), True,
        )
        self.assertEqual(
            report["serial_sha256"],
            hashlib.sha256((directory / "serial.log").read_bytes()).hexdigest(),
        )

    def test_returned_json_arrays_are_associated_with_in_memory_tuples(self):
        result = self.result("panic")
        result["evidence"] = {"selected_sectors": SECTORS}
        directory = self.write_mode("panic", result)
        report = harness_report.write_report(
            directory / "harness.json", str(uuid.uuid4()), "panic", "panic", result, True,
        )
        self.assertEqual(report["observed_outcome"], "panic")

    def test_block_and_aggregate_success_results_keep_native_shapes(self):
        block = self.result("success")
        block["block"] = {"selected_sectors": SECTORS, "separate_vm_boots": 2}
        block_dir = self.write_mode("block-user", block)
        self.assertTrue(
            harness_report.write_report(
                block_dir / "harness.json", str(uuid.uuid4()), "block-user", "success", block, True,
            )["harness_passed"]
        )
        for mode, key in (("terminal-test", "terminal"), ("recovery-test", "recovery")):
            aggregate = self.result("success")
            # Native terminal/recovery aggregates intentionally omit the
            # timeout budget; the producer binds their identities without
            # inventing fields that the importer will continue to reject.
            aggregate.pop("timeout_seconds")
            aggregate[key] = {"verified": True, "boots": 2}
            directory = self.write_mode(mode, aggregate)
            self.assertTrue(
                harness_report.write_report(
                    directory / "harness.json", str(uuid.uuid4()), mode, "success", aggregate, True,
                )["harness_passed"]
            )

    def test_suite_clears_stale_reports_and_retains_partial_failure(self):
        output = self.root / "boot"
        output.mkdir()
        for mode in ("panic", "hang", "skipped"):
            stale = output / mode
            stale.mkdir()
            (stale / "harness.json").write_text("stale")

        panic = self.result("panic")
        hang = self.result("timeout", timed_out=True)
        for mode, result, serial in (("panic", panic, "fixture\n"), ("hang", hang, "loader output\n")):
            directory = output / mode
            directory.mkdir(exist_ok=True)
            (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            (directory / "serial.log").write_text(serial)

        def fake_build(mode):
            return output / mode / "rustic-os.img"

        def fake_run(image, timeout):
            del timeout
            return json.loads((Path(image).parent / "result.json").read_text())

        with mock.patch.object(runner, "OUTPUT", output), \
             mock.patch.object(runner, "EXPECTED", {"panic": "panic", "hang": "timeout"}), \
             mock.patch.object(runner, "build", side_effect=fake_build), \
             mock.patch.object(runner, "run", side_effect=fake_run), \
             mock.patch.object(runner, "reached", side_effect=lambda mode, serial: mode == "panic"):
            with self.assertRaises(RuntimeError):
                runner.suite(3)

        panic_report = json.loads((output / "panic" / "harness.json").read_text())
        hang_report = json.loads((output / "hang" / "harness.json").read_text())
        self.assertTrue(panic_report["harness_passed"])
        self.assertFalse(hang_report["harness_passed"])
        self.assertEqual(panic_report["suite_run_id"], hang_report["suite_run_id"])
        self.assertEqual(str(uuid.UUID(panic_report["suite_run_id"])), panic_report["suite_run_id"])
        self.assertFalse((output / "skipped" / "harness.json").exists())
        self.assertFalse((output / "suite.json").exists())

    def test_suite_pre_result_exception_leaves_mode_unknown(self):
        output = self.root / "boot"
        stale = output / "panic"
        stale.mkdir(parents=True)
        (stale / "harness.json").write_text("stale")

        with mock.patch.object(runner, "OUTPUT", output), \
             mock.patch.object(runner, "EXPECTED", {"panic": "panic"}), \
             mock.patch.object(runner, "build", return_value=output / "panic" / "image"), \
             mock.patch.object(runner, "run", side_effect=RuntimeError("executor failed")):
            with self.assertRaisesRegex(RuntimeError, "executor failed"):
                runner.suite(3)

        self.assertFalse((stale / "harness.json").exists())
        self.assertFalse((output / "suite.json").exists())


if __name__ == "__main__":
    unittest.main()
