# SPDX-License-Identifier: Apache-2.0
"""Focused host-only tests for bounded failure triage."""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools.research.failure_triage import build_request, diagnose_request
from tools.research.failure_triage import diagnose as diagnosis
from tools.research.failure_triage.baseline import classify
from tools.research.failure_triage.format import MAX_REQUEST_BYTES, RequestError, serialize_json
from tools.research.failure_triage.schema import validate_request, validate_response
from tools.research.decisions_transport import TransportError


class FailureTriageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.manifest_path = self.root / "manifest.json"
        self.log_path = self.root / "run.log"
        self.diff_path = self.root / "change.diff"
        self.manifest = {
            "schema_version": 1,
            "run_id": "run-1",
            "runner": "host",
            "facts": {
                "returncode": 1,
                "outcome": "failed",
                "harness_passed": False,
                "observed": {"owner": "fixture", "count": 2},
                "command": "$(touch should-not-execute)",
            },
        }
        self.manifest_path.write_text(json.dumps(self.manifest), encoding="utf-8")
        self.log_path.write_text("assertion failed: expected 1 got 2\n", encoding="utf-8")
        self.diff_path.write_text("+ compiler error example only\n", encoding="utf-8")

    def tearDown(self):
        self.temp.cleanup()

    def _request(self, *, facts=None, include_diff=False):
        if facts is not None:
            manifest = copy.deepcopy(self.manifest)
            manifest["facts"] = facts
            self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        return build_request(
            self.manifest_path,
            [f"{self.log_path}:1:1"],
            [f"{self.diff_path}:1:1"] if include_diff else [],
        )

    def test_prepare_preserves_facts_and_marks_diff_without_execution(self):
        request = self._request(include_diff=True)
        self.assertEqual(request["state"]["facts"], self.manifest["facts"])
        self.assertEqual([row["id"] for row in request["state"]["excerpts"]], ["excerpt-001", "excerpt-002"])
        self.assertEqual([row["kind"] for row in request["state"]["excerpts"]], ["log", "diff"])
        self.assertFalse((self.root / "should-not-execute").exists())
        self.assertEqual(request["questions"]["classification"]["type"], "choice")
        self.assertEqual(request["questions"]["excerpt-001"]["type"], "noul")

    def test_duplicate_and_nonfinite_manifest_json_is_rejected(self):
        self.manifest_path.write_bytes(
            b'{"schema_version":1,"run_id":"r","runner":"host","facts":{},"facts":{}}'
        )
        with self.assertRaises(RequestError):
            build_request(self.manifest_path, [f"{self.log_path}:1:1"])
        self.manifest_path.write_bytes(
            b'{"schema_version":1,"run_id":"r","runner":"host","facts":{"value":NaN}}'
        )
        with self.assertRaises(RequestError):
            build_request(self.manifest_path, [f"{self.log_path}:1:1"])

    def test_request_rejects_inconsistent_facts_and_range_or_hash_mutation(self):
        request = self._request()
        inconsistent = copy.deepcopy(request)
        inconsistent["state"]["facts"] = dict(inconsistent["state"]["facts"], returncode=True)
        with self.assertRaises(RequestError):
            validate_request(inconsistent)
        bad_hash = copy.deepcopy(request)
        bad_hash["state"]["excerpts"][0]["sha256"] = "0" * 64
        with self.assertRaises(RequestError):
            validate_request(bad_hash)
        bad_range = copy.deepcopy(request)
        bad_range["state"]["excerpts"][0]["end"] = 2
        with self.assertRaises(RequestError):
            validate_request(bad_range)

    def test_expected_negative_preserved_and_harness_pass_forces_unknown(self):
        facts = {
            "returncode": 35,
            "outcome": "panic",
            "expected_outcome": "panic",
            "harness_passed": False,
        }
        request = self._request(facts=facts)
        request_path = self.root / "request.json"
        report_path = self.root / "report.json"
        request_path.write_bytes(serialize_json(request))
        status, reason = diagnose_request(request_path, report_path)
        report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual((status, reason), ("baseline", None))
        self.assertEqual(report["facts"], facts)
        self.assertEqual(report["hypothesis"]["category"], "assertion")
        passed = dict(facts, harness_passed=True)
        request = self._request(facts=passed)
        request_path.write_bytes(serialize_json(request))
        diagnose_request(request_path, report_path)
        report = json.loads(report_path.read_text(encoding="utf-8"))
        self.assertEqual(report["hypothesis"], {"category": "unknown", "confidence": 0.0, "accepted": False})
        self.assertEqual(report["facts"], passed)

    def test_oversize_and_canonical_request_bounds(self):
        self.log_path.write_text("x" * MAX_REQUEST_BYTES, encoding="utf-8")
        with self.assertRaises(RequestError):
            build_request(self.manifest_path, [f"{self.log_path}:1:1"])
        self.log_path.write_text("assertion failed\n", encoding="utf-8")
        request = self._request()
        expanded = copy.deepcopy(request)
        expanded["state"]["facts"]["numbers"] = [10.0] * 12_000
        # Simulate a compact raw spelling smaller than canonical output; both
        # representations are bounded independently by validate_request.
        with self.assertRaises(RequestError):
            validate_request(expanded, serialized_size=1)

    def test_optional_choice_confidence_and_probabilities_validate(self):
        request = self._request()
        answer_ids = {"classification", "excerpt-001"}
        base = {
            "answers": {
                "classification": {"type": "choice", "choice": "assertion"},
                "excerpt-001": {"type": "noul", "noul": 0.75},
            },
            "model": "typesafe/jev-1.13-20260917",
            "usage": {"input_tokens": 2, "output_tokens": 1},
        }
        validate_response(base, answer_ids)
        confident = copy.deepcopy(base)
        confident["answers"]["classification"].update(
            confidence=0.9,
            probabilities={"assertion": 0.9, "unknown": 0.1},
        )
        validate_response(confident, answer_ids)
        invalid = copy.deepcopy(confident)
        invalid["answers"]["classification"]["choice"] = "not-a-choice"
        with self.assertRaises(RequestError):
            validate_response(invalid, answer_ids)
        invalid = copy.deepcopy(confident)
        invalid["answers"]["classification"]["probabilities"]["assertion"] = float("nan")
        with self.assertRaises(RequestError):
            validate_response(invalid, answer_ids)

    def test_live_optional_confidence_and_outage_fallback_are_safe(self):
        request = self._request()
        request_path = self.root / "request.json"
        request_path.write_bytes(serialize_json(request))
        output_path = self.root / "live.json"
        response = {
            "answers": {
                "classification": {"type": "choice", "choice": "assertion"},
                "excerpt-001": {"type": "noul", "noul": 0.8},
            },
            "model": "typesafe/jev-1.13-20260917",
            "provider": "TypeSafe",
            "usage": {"input_tokens": 2, "output_tokens": 1},
        }
        with mock.patch.object(diagnosis, "api_key", return_value="secret"), mock.patch.object(
            diagnosis, "post_json", return_value=json.dumps(response).encode("utf-8")
        ):
            status, reason = diagnose_request(request_path, output_path, live=True)
        self.assertEqual((status, reason), ("live", None))
        report = json.loads(output_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "live")
        self.assertEqual(report["hypothesis"]["confidence"], None)
        self.assertFalse(report["hypothesis"]["accepted"])
        self.assertEqual(report["model_hypothesis"]["category"], "assertion")
        with mock.patch.object(diagnosis, "api_key", return_value="secret"), mock.patch.object(
            diagnosis, "post_json", side_effect=TransportError("Authorization Bearer secret")
        ):
            status, reason = diagnose_request(request_path, output_path, live=True)
        self.assertEqual((status, reason), ("unavailable", "transport TransportError"))
        report_text = output_path.read_text(encoding="utf-8")
        self.assertNotIn("secret", report_text)
        self.assertEqual(json.loads(report_text)["fallback"], "baseline")

    def test_unverified_live_run_keeps_raw_hypothesis_but_abstains(self):
        for association in ('unverified', 'unknown', 'unavailable'):
            with self.subTest(association=association):
                request = self._request(facts={'run_association': association})
                path = self.root / 'request.json'
                path.write_bytes(serialize_json(request))
                output = self.root / 'report.json'
                response = {'answers': {
                    'classification': {'type': 'choice', 'choice': 'assertion', 'confidence': .99},
                    'excerpt-001': {'type': 'noul', 'noul': .9}},
                    'model': 'typesafe/jev-1.13',
                    'usage': {'input_tokens': 1, 'output_tokens': 1}}
                with mock.patch.object(diagnosis, 'api_key', return_value='secret'), \
                     mock.patch.object(diagnosis, 'post_json', return_value=serialize_json(response)):
                    diagnose_request(path, output, live=True)
                report = json.loads(output.read_text())
                self.assertEqual(report['guard'], 'run_association_unverified')
                self.assertFalse(report['hypothesis']['accepted'])
                self.assertTrue(report['model_hypothesis']['accepted'])

    def test_observed_category_wins_and_all_matching_logs_rank_first(self):
        request = self._request(facts={'failure_category': 'resource_limit', 'status': 'resource_limit'})
        excerpts = request['state']['excerpts']
        excerpts[0]['text'] = 'error[E0308]: mismatched types\n'
        for number in (2, 3):
            excerpts.append({**excerpts[0], 'id': f'excerpt-{number:03d}',
                             'text': 'out of memory: resource limit exceeded\n'})
        result = classify(request['state']['facts'], excerpts)
        self.assertEqual(result['hypothesis']['category'], 'resource_limit')
        self.assertTrue(result['hypothesis']['accepted'])
        self.assertIsNone(result['hypothesis']['confidence'])
        self.assertEqual([r['id'] for r in result['evidence'][:2]], ['excerpt-002', 'excerpt-003'])

    def test_malformed_provider_json_preserves_fallback(self):
        path = self.root / 'request.json'
        path.write_bytes(serialize_json(self._request()))
        output = self.root / 'report.json'
        for response in (b'{', b'{"a":1,"a":2}', b'{"a":' + b'9' * 5000 + b'}'):
            with self.subTest(response_length=len(response)), \
                 mock.patch.object(diagnosis, 'api_key', return_value='secret'), \
                 mock.patch.object(diagnosis, 'post_json', return_value=response):
                status, reason = diagnose_request(path, output, live=True)
                self.assertEqual((status, reason), ('unavailable', 'invalid_response'))
                report = json.loads(output.read_text())
                self.assertEqual(report['fallback'], 'baseline')
                self.assertEqual(report['facts'], self.manifest['facts'])

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'requires POSIX FIFO')
    def test_special_files_are_refused_without_waiting_for_peer(self):
        fifo = self.root / 'pipe'
        os.mkfifo(fifo)
        script = """from pathlib import Path
import sys
from tools.research.failure_triage.format import read_bounded_bytes, write_bounded_bytes, RequestError
from tools.research.decisions_transport import _read_credential_bytes, TransportError
path = Path(sys.argv[1])
for operation in (lambda: read_bounded_bytes(path), lambda: write_bounded_bytes(path, b'x'), lambda: _read_credential_bytes(path)):
    try:
        operation()
    except (RequestError, TransportError):
        continue
    raise AssertionError('special file accepted')
"""
        subprocess.run([sys.executable, '-c', script, str(fifo)], check=True, timeout=2,
                       capture_output=True)
        link = self.root / 'link'
        link.symlink_to(self.log_path)
        with self.assertRaises(RequestError):
            build_request(self.manifest_path, [f'{link}:1:1'])


if __name__ == "__main__":
    unittest.main()
