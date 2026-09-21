# SPDX-License-Identifier: Apache-2.0
"""Focused tests for explicit boot harness import and triage guards."""

from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools.boot_support import harness_report
from tools.research.failure_triage import build_request, diagnose_request
from tools.research.failure_triage.format import RequestError, serialize_json, write_json
from tools.research.failure_triage.import_report import build_manifest, write_manifest


class TriageHarnessTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source_number = 0

    @staticmethod
    def result(outcome: str = "panic") -> dict[str, object]:
        return {
            "outcome": outcome,
            "returncode": 35 if outcome == "panic" else 33,
            "timed_out": False,
            "elapsed_seconds": 1.0,
            "timeout_seconds": 5,
            "build_id": "build-1",
            "image_sha256": "b" * 64,
        }

    def source_pair(self, *, reached: bool = True, outcome: str = "panic"):
        result = self.result(outcome)
        self.source_number += 1
        directory = self.root / f"boot-{self.source_number}"
        directory.mkdir()
        (directory / "result.json").write_bytes(json.dumps(result, indent=2).encode() + b"\n")
        (directory / "serial.log").write_bytes(b"assertion failed: fixture output\n")
        harness_report.write_report(
            directory / "harness.json",
            "00000000-0000-4000-8000-000000000001",
            "panic",
            outcome,
            result,
            reached,
        )
        return directory, result

    def test_import_promotes_bound_acceptance_and_namespaced_provenance(self):
        directory, result = self.source_pair()
        manifest = build_manifest(
            "boot", directory / "result.json", "caller-label",
            harness_path=directory / "harness.json",
        )
        facts = manifest["facts"]
        self.assertEqual(facts["harness_passed"], True)
        self.assertEqual(facts["expected_outcome"], "panic")
        self.assertEqual(facts["harness_suite_run_id"], "00000000-0000-4000-8000-000000000001")
        self.assertEqual(facts["harness_mode"], "panic")
        self.assertEqual(facts["harness_report"]["observed_outcome"], result["outcome"])
        self.assertEqual(
            facts["source_harness"]["sha256"],
            hashlib.sha256((directory / "harness.json").read_bytes()).hexdigest(),
        )
        self.assertEqual(facts["native_report"], result)

    def test_stale_hash_metadata_and_contradictory_acceptance_are_rejected(self):
        directory, _ = self.source_pair()
        original = json.loads((directory / "harness.json").read_text())
        mutations = {
            "result_sha256": "0" * 64,
            "producer": "other/v1",
            "build_id": "different-build",
            "image_sha256": "c" * 64,
            "observed_outcome": "success",
            "harness_passed": False,
            "reached_fixture": "yes",
        }
        for field, value in mutations.items():
            with self.subTest(field=field):
                changed = copy.deepcopy(original)
                changed[field] = value
                (directory / "harness.json").write_bytes(serialize_json(changed))
                with self.assertRaises(RequestError):
                    build_manifest(
                        "boot", directory / "result.json", "r",
                        harness_path=directory / "harness.json",
                    )
        (directory / "harness.json").write_bytes(serialize_json(original))

    def test_output_aliases_are_rejected_during_build_and_write(self):
        directory, _ = self.source_pair()
        native = directory / "result.json"
        harness = directory / "harness.json"
        with self.assertRaises(RequestError):
            build_manifest("boot", native, "r", harness_path=harness, output_path=native)
        with self.assertRaises(RequestError):
            build_manifest("boot", native, "r", harness_path=harness, output_path=harness)

        manifest = build_manifest("boot", native, "r", harness_path=harness)
        with self.assertRaises(RequestError):
            write_manifest(native, manifest)
        with self.assertRaises(RequestError):
            write_manifest(harness, manifest)

    def test_harness_attachment_is_boot_only_and_existing_boot_limits_remain(self):
        directory, _ = self.source_pair()
        sandbox = {
            "schema_version": 1,
            "job_id": "job",
            "revision": "a" * 40,
            "status": "success",
        }
        sandbox_path = self.root / "sandbox.json"
        sandbox_path.write_bytes(serialize_json(sandbox))
        with self.assertRaises(RequestError):
            build_manifest(
                "sandbox", sandbox_path, "r", harness_path=directory / "harness.json"
            )
        no_timeout = self.result()
        no_timeout.pop("timeout_seconds")
        native = self.root / "terminal.json"
        native.write_bytes(serialize_json(no_timeout))
        with self.assertRaises(RequestError):
            build_manifest("boot", native, "r", harness_path=directory / "harness.json")

    def _diagnosis(self, *, reached: bool):
        directory, _ = self.source_pair(reached=reached)
        manifest_path = directory / "manifest.json"
        manifest = build_manifest(
            "boot", directory / "result.json", "r", harness_path=directory / "harness.json"
        )
        write_manifest(manifest_path, manifest)
        request = build_request(manifest_path, [f"{directory / 'serial.log'}:1:1"])
        request_path = directory / "request.json"
        write_json(request_path, request, pretty=False)
        output = directory / "diagnosis.json"
        response = {
            "answers": {
                "classification": {
                    "type": "choice", "choice": "assertion", "confidence": 0.99,
                },
                "excerpt-001": {"type": "noul", "noul": 0.95},
            },
            "model": "typesafe/jev-1.13",
            "usage": {"input_tokens": 1, "output_tokens": 1},
        }
        with mock.patch("tools.research.failure_triage.diagnose.api_key", return_value="secret"), \
             mock.patch("tools.research.failure_triage.diagnose.post_json", return_value=serialize_json(response)):
            status, reason = diagnose_request(request_path, output, live=True)
        return status, reason, json.loads(output.read_text())

    def test_high_confidence_live_diagnosis_is_suppressed_only_for_true_acceptance(self):
        status, reason, report = self._diagnosis(reached=True)
        self.assertEqual((status, reason), ("live", None))
        self.assertEqual(report["guard"], "harness_passed")
        self.assertFalse(report["hypothesis"]["accepted"])
        self.assertTrue(report["model_hypothesis"]["accepted"])

        status, reason, report = self._diagnosis(reached=False)
        self.assertEqual((status, reason), ("live", None))
        self.assertIsNone(report.get("guard"))
        self.assertTrue(report["hypothesis"]["accepted"])
        self.assertTrue(report["facts"]["harness_passed"] is False)


if __name__ == "__main__":
    unittest.main()
