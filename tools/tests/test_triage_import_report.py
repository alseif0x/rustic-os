# SPDX-License-Identifier: Apache-2.0
"""Focused host tests for bounded native-report import."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

from tools.research.failure_triage.format import MAX_REQUEST_BYTES, RequestError, serialize_json
from tools.research.failure_triage.import_report import build_manifest


class NativeReportImporterTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def write_report(self, value, name="native.json"):
        path = self.root / name
        path.write_bytes(serialize_json(value))
        return path

    def test_boot_promotes_observations_preserves_source_and_never_promotes_reserved_fields(self):
        report = {
            "outcome": "panic",
            "returncode": 35,
            "timed_out": False,
            "build_id": "a" * 16,
            "image_sha256": "b" * 64,
            "elapsed_seconds": 1.25,
            "timeout_seconds": 45,
            "memory_mib": 256,
            "command": ["touch", "should-not-run"],
            "harness_passed": True,
            "failure_category": "executor",
            "unknown": {"value": 3},
        }
        source = self.write_report(report)
        manifest = build_manifest("boot", source, "case-1")
        facts = manifest["facts"]

        self.assertEqual(manifest.keys(), {"schema_version", "run_id", "runner", "facts"})
        self.assertEqual(manifest["runner"], "boot")
        self.assertEqual(facts["outcome"], "panic")
        self.assertEqual(facts["returncode"], 35)
        self.assertEqual(facts["memory_mib"], 256)
        self.assertEqual(facts["native_report"], report)
        self.assertNotIn("command", facts)
        self.assertNotIn("harness_passed", facts)
        self.assertNotIn("failure_category", facts)
        self.assertEqual(
            facts["source_report"],
            {"kind": "boot", "path": str(source), "sha256": hashlib.sha256(source.read_bytes()).hexdigest()},
        )

    def test_boot_rejects_bad_types_timeout_mismatch_and_hashes(self):
        base = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "build_id": "build",
            "image_sha256": "c" * 64,
            "elapsed_seconds": 0.1,
            "timeout_seconds": 45.0,
        }
        cases = [
            {**base, "timed_out": 1},
            {**base, "timed_out": True},
            {**base, "outcome": "timeout"},
            {**base, "image_sha256": "not-a-hash"},
            {**base, "elapsed_seconds": float("nan")},
            {**base, "returncode": True},
            {**base, "outcome": "made-up-policy"},
        ]
        for index, value in enumerate(cases):
            with self.subTest(index=index), self.assertRaises(RequestError):
                build_manifest("boot", self.write_report(value, f"bad-{index}.json"), "case")

    def test_sandbox_outer_status_and_nested_guest_result_stay_distinct(self):
        report = {
            "schema_version": 1,
            "job_id": "job-1",
            "revision": "d" * 40,
            "status": "boot_failed",
            "mode": "panic",
            "worker_exit_code": 23,
            "guest_result": {
                "outcome": "panic",
                "returncode": 35,
                "harness_passed": True,
            },
            "harness_passed": True,
            "command": "ignored",
        }
        manifest = build_manifest("sandbox", self.write_report(report), "sandbox-case")
        facts = manifest["facts"]
        self.assertEqual(manifest["runner"], "sandbox")
        self.assertEqual(facts["status"], "boot_failed")
        self.assertEqual(facts["worker_exit_code"], 23)
        self.assertEqual(facts["guest_result"], report["guest_result"])
        self.assertNotIn("returncode", facts)
        self.assertNotIn("outcome", facts)
        self.assertNotIn("harness_passed", facts)
        for status in (
            "success",
            "build_failed",
            "boot_failed",
            "build_timeout",
            "boot_timeout",
            "resource_limit",
            "executor_error",
            "cleanup_failed",
            "cancelled",
        ):
            report["status"] = status
            self.assertEqual(build_manifest("sandbox", self.write_report(report, f"{status}.json"), "r")["facts"]["status"], status)

    def test_github_job_requires_completed_and_maps_terminal_conclusion(self):
        report = {
            "status": "completed",
            "id": 123,
            "run_id": 456,
            "name": "unit tests",
            "conclusion": "timed_out",
            "head_sha": "e" * 40,
            "harness_passed": True,
            "failure_category": "timeout",
        }
        manifest = build_manifest("github-job", self.write_report(report), "github-case")
        facts = manifest["facts"]
        self.assertEqual(manifest["runner"], "host")
        self.assertEqual(facts["status"], "timed_out")
        self.assertTrue(facts["timed_out"])
        self.assertEqual(facts["source_job_id"], 123)
        self.assertEqual(facts["source_run_id"], 456)
        self.assertEqual(facts["source_name"], "unit tests")
        self.assertNotIn("harness_passed", facts)
        self.assertNotIn("failure_category", facts)

        report["conclusion"] = "success"
        manifest = build_manifest("github-job", self.write_report(report, "success.json"), "github-case")
        self.assertNotIn("timed_out", manifest["facts"])

        report["status"] = "in_progress"
        with self.assertRaises(RequestError):
            build_manifest("github-job", self.write_report(report, "running.json"), "github-case")

    def test_bounded_regular_inputs_and_output_aliases_are_refused(self):
        report = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "build_id": "build",
            "image_sha256": "f" * 64,
            "elapsed_seconds": 1,
            "timeout_seconds": 2,
        }
        source = self.write_report(report)
        output = self.root / "manifest.json"
        with self.assertRaises(RequestError):
            build_manifest("boot", source, "r", output_path=source)

        hardlink = self.root / "hardlink.json"
        os.link(source, hardlink)
        with self.assertRaises(RequestError):
            build_manifest("boot", source, "r", output_path=hardlink)

        oversized = self.root / "oversized.json"
        oversized.write_bytes(b"{" + b'"x":"' + b"a" * MAX_REQUEST_BYTES + b'"}')
        with self.assertRaises(RequestError):
            build_manifest("boot", oversized, "r")

        if hasattr(os, "mkfifo"):
            fifo = self.root / "input.pipe"
            os.mkfifo(fifo)
            with self.assertRaises(RequestError):
                build_manifest("boot", fifo, "r")

    def test_final_manifest_size_is_bounded_after_native_report_duplication(self):
        report = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "build_id": "build",
            "image_sha256": "1" * 64,
            "elapsed_seconds": 1,
            "timeout_seconds": 2,
            "padding": "x" * (MAX_REQUEST_BYTES - 250),
        }
        source = self.write_report(report)
        # The source itself fits the input ceiling, but retaining it with the
        # selected facts and provenance must still fit the portable manifest.
        self.assertLessEqual(len(source.read_bytes()), MAX_REQUEST_BYTES)
        with self.assertRaises(RequestError):
            build_manifest("boot", source, "r")


if __name__ == "__main__":
    unittest.main()
