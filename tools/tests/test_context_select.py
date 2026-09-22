# SPDX-License-Identifier: Apache-2.0
"""Focused host tests for the bounded JEV context-ranking pilot."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
import urllib.error

from tools.research.context_select import build_request
from tools.research.context_select.format import MAX_CANDIDATES, MAX_REQUEST_BYTES, RequestError, serialize_json
from tools.research.context_select import rank as ranking
from tools.research.context_select import transport


class ContextSelectTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "config", "user.email", "test@example.invalid"], check=True)
        subprocess.run(["git", "-C", str(self.root), "config", "user.name", "Context Test"], check=True)
        self.source = self.root / "source.txt"
        self.source.write_text("alpha kernel\nbeta policy\ngamma kernel\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(self.root), "add", "source.txt"], check=True)
        subprocess.run(["git", "-C", str(self.root), "commit", "-qm", "fixture"], check=True)

    def tearDown(self):
        self.temp.cleanup()

    def _request(self, *ranges: str, task: str = "kernel policy"):
        return build_request(task, ranges, root=self.root)

    def test_prepare_is_explicit_inclusive_and_question_ids_are_instructions(self):
        request = self._request("source.txt:1:2")
        candidate = request["state"]["candidates"][0]
        self.assertEqual(candidate["text"], "alpha kernel\nbeta policy\n")
        self.assertEqual(candidate["start"], 1)
        self.assertEqual(candidate["end"], 2)
        self.assertEqual(set(request["questions"]), {candidate["id"]})
        question = request["questions"][candidate["id"]]
        self.assertIn(candidate["id"], question["instructions"])
        self.assertEqual(question["type"], "noul")
        self.assertEqual(set(request), {"model", "state", "questions"})

    def test_prepare_rejects_duplicate_untracked_outside_symlink_and_bad_range(self):
        with self.assertRaises(RequestError):
            self._request("source.txt:1:1", "source.txt:1:1")
        untracked = self.root / "untracked.txt"
        untracked.write_text("no\n", encoding="utf-8")
        with self.assertRaises(RequestError):
            self._request("untracked.txt:1:1")
        outside = Path(self.temp.name).parent / (Path(self.temp.name).name + "-outside")
        outside.write_text("outside\n", encoding="utf-8")
        try:
            with self.assertRaises(RequestError):
                self._request(f"{outside}:1:1")
        finally:
            outside.unlink()
        link = self.root / "link.txt"
        link.symlink_to(self.source)
        with self.assertRaises(RequestError):
            self._request("link.txt:1:1")
        with self.assertRaises(RequestError):
            self._request("source.txt:2:1")
        with self.assertRaises(RequestError):
            self._request("source.txt:1:9")

    def test_prepare_enforces_candidate_and_serialized_byte_bounds(self):
        paths = []
        for index in range(MAX_CANDIDATES + 1):
            path = self.root / f"candidate-{index}.txt"
            path.write_text(f"candidate {index}\n", encoding="utf-8")
            paths.append(path.name + ":1:1")
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)
        subprocess.run(["git", "-C", str(self.root), "commit", "-qm", "bounds"], check=True)
        with self.assertRaises(RequestError):
            self._request(*paths)
        with self.assertRaises(RequestError):
            self._request("source.txt:1:1", task="x" * MAX_REQUEST_BYTES)

    def test_default_rank_is_offline_and_retains_all_candidates_with_stable_ties(self):
        request = self._request("source.txt:1:1", "source.txt:2:2", "source.txt:3:3", task="absent")
        request_path = self.root / "request.json"
        output_path = self.root / "baseline.json"
        request_path.write_bytes(serialize_json(request))
        with mock.patch.object(ranking, "post_json", side_effect=AssertionError("network")):
            status, reason = ranking.rank_request(request_path, output_path)
        self.assertEqual((status, reason), ("baseline", None))
        report = json.loads(output_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "baseline")
        self.assertEqual([row["id"] for row in report["ranking"]], ["candidate-001", "candidate-002", "candidate-003"])
        self.assertTrue(all("text" in row and "sha256" in row for row in report["ranking"]))
        self.assertTrue(all("noul" not in row for row in report["ranking"]))

    def test_response_validation_rejects_missing_extra_bool_and_out_of_range(self):
        ids = {"candidate-001"}
        base = {"answers": {"candidate-001": {"type": "noul", "noul": 0.5}}, "model": "typesafe/jev-1.13", "usage": {"input_tokens": 1, "output_tokens": 1}}
        ranking.validate_response(base, ids)
        for mutate in (
            lambda value: value["answers"].pop("candidate-001"),
            lambda value: value["answers"].update(extra={"type": "noul", "noul": 0.1}),
            lambda value: value["answers"]["candidate-001"].update(noul=True),
            lambda value: value["answers"]["candidate-001"].update(noul=1.1),
            lambda value: value["usage"].update(unexpected=1),
        ):
            candidate = json.loads(json.dumps(base))
            mutate(candidate)
            with self.assertRaises(ranking.ResponseError):
                ranking.validate_response(candidate, ids)

    def test_live_invalid_response_and_credential_reason_are_sanitized(self):
        request = self._request("source.txt:1:1")
        request_path = self.root / "request.json"
        output_path = self.root / "report.json"
        request_path.write_bytes(serialize_json(request))
        with mock.patch.object(ranking, "post_json", side_effect=transport.TransportError("Authorization Bearer super-secret")), mock.patch.object(
            ranking, "api_key", return_value="super-secret"
        ):
            status, reason = ranking.rank_request(request_path, output_path, live=True, explicit_key_file=True, key_file=self.root / "key")
        self.assertEqual(status, "unavailable")
        self.assertEqual(reason, "transport TransportError")
        report_text = output_path.read_text(encoding="utf-8")
        self.assertNotIn("super-secret", report_text)
        self.assertEqual(json.loads(report_text)["fallback"], "baseline")

    def test_live_wire_posts_once_to_native_endpoint_and_retains_numeric_answers(self):
        request = self._request("source.txt:1:1", "source.txt:2:2")
        request_path = self.root / "request.json"
        output_path = self.root / "live.json"
        request_path.write_bytes(serialize_json(request))
        response = {
            "answers": {
                "candidate-001": {"type": "noul", "noul": 0},
                "candidate-002": {"type": "noul", "noul": 1},
            },
            "model": "typesafe/jev-1.13-20260917",
            "provider": "TypeSafe",
            "usage": {"input_tokens": 7, "output_tokens": 2, "cost": 0.01},
        }
        calls = []

        class FakeResponse:
            def __init__(self, data):
                self.data = data

            def read(self, limit):
                self.limit = limit
                return self.data

            def close(self):
                pass

        class FakeOpener:
            def open(self, request, timeout):
                calls.append((request, timeout))
                return FakeResponse(json.dumps(response).encode("utf-8"))

        with mock.patch.dict("os.environ", {"OPENROUTER_API_KEY": "test-key"}), mock.patch.object(
            transport.urllib.request, "build_opener", return_value=FakeOpener()
        ):
            status, reason = ranking.rank_request(request_path, output_path, live=True)
        self.assertEqual((status, reason), ("live", None))
        self.assertEqual(len(calls), 1)
        sent_request, timeout = calls[0]
        self.assertEqual(sent_request.full_url, transport.DECISIONS_URL)
        self.assertEqual(sent_request.get_method(), "POST")
        self.assertEqual(timeout, transport.REQUEST_TIMEOUT_SECONDS)
        self.assertEqual(sent_request.headers["Authorization"], "Bearer test-key")
        sent = json.loads(sent_request.data.decode("utf-8"))
        self.assertEqual(sent["model"], "typesafe/jev-1.13")
        self.assertEqual(set(sent["questions"]), {"candidate-001", "candidate-002"})
        report = json.loads(output_path.read_text(encoding="utf-8"))
        self.assertEqual(report["status"], "live")
        self.assertEqual(report["model"], response["model"])
        self.assertEqual([row["id"] for row in report["ranking"]], ["candidate-002", "candidate-001"])
        self.assertEqual({row["text"] for row in report["ranking"]}, {"alpha kernel\n", "beta policy\n"})

    def test_http_failures_and_surrogate_response_save_sanitized_baseline(self):
        request = self._request("source.txt:1:1")
        request_path = self.root / "request.json"
        request_path.write_bytes(serialize_json(request))
        for failure, expected in (
            (urllib.error.HTTPError(transport.DECISIONS_URL, 401, "secret body", {}, None), "HTTP 401"),
            (urllib.error.HTTPError(transport.DECISIONS_URL, 429, "secret body", {}, None), "HTTP 429"),
            (TimeoutError("Bearer super-secret"), "transport TimeoutError"),
        ):
            with self.subTest(expected=expected):
                output_path = self.root / f"failure-{expected.replace(' ', '-')}.json"
                class FailedOpener:
                    def open(self, request, timeout):
                        raise failure

                with mock.patch.dict("os.environ", {"OPENROUTER_API_KEY": "super-secret"}), mock.patch.object(
                    transport.urllib.request, "build_opener", return_value=FailedOpener()
                ):
                    status, reason = ranking.rank_request(request_path, output_path, live=True)
                self.assertEqual((status, reason), ("unavailable", expected))
                report_text = output_path.read_text(encoding="utf-8")
                self.assertNotIn("super-secret", report_text)
                self.assertEqual(json.loads(report_text)["fallback"], "baseline")
                if isinstance(failure, urllib.error.HTTPError):
                    failure.close()

        output_path = self.root / "surrogate.json"
        invalid = b'{"answers":{"candidate-001":{"type":"noul","noul":0.5}},"model":"\\ud800","usage":{"input_tokens":1,"output_tokens":1}}'

        class FakeResponse:
            def read(self, limit):
                return invalid

            def close(self):
                pass

        class FakeOpener:
            def open(self, request, timeout):
                return FakeResponse()

        with mock.patch.dict("os.environ", {"OPENROUTER_API_KEY": "super-secret"}), mock.patch.object(
            transport.urllib.request, "build_opener", return_value=FakeOpener()
        ):
            status, reason = ranking.rank_request(request_path, output_path, live=True)
        self.assertEqual((status, reason), ("unavailable", "invalid_response"))
        self.assertTrue(output_path.exists())
        self.assertEqual(json.loads(output_path.read_text(encoding="utf-8"))["fallback"], "baseline")

    def test_key_file_is_data_and_redirect_handler_refuses(self):
        key_file = self.root / "key.env"
        marker = self.root / "executed"
        key_file.write_text("OPENROUTER_API_KEY=$(touch-marker)\n", encoding="utf-8")
        self.assertEqual(transport.api_key(key_file=key_file, explicit_key_file=True), "$(touch-marker)")
        self.assertFalse(marker.exists())
        handler = transport._NoRedirect()
        with self.assertRaises(transport.TransportError):
            handler.redirect_request(None, None, 302, "Found", {}, "https://example.invalid")


if __name__ == "__main__":
    unittest.main()
