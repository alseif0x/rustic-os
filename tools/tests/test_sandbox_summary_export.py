# SPDX-License-Identifier: Apache-2.0
"""Completed recovery reports remain bounded while crossing the real host collector."""
from contextlib import ExitStack
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from sandbox_support import artifacts, jobs


class CompletedSummaryExport(unittest.TestCase):
    def execute(self, mode, summary_sizes):
        """Fake the container transport; keep job orchestration and collection real."""
        transfers = []

        def worker_command(arguments, log, **kwargs):
            log.write_bytes(b"synthetic worker success")
            return 0

        def transfer(arguments, destination, **kwargs):
            name = arguments[-1]
            payload = b"{}"
            if name == "result.json":
                payload = json.dumps({"outcome": "success"}).encode()
            payload += b" " * (summary_sizes.get(name, len(payload)) - len(payload))
            output = io.BytesIO()
            with tarfile.open(fileobj=output, mode="w") as archive:
                member = tarfile.TarInfo(name)
                member.size = len(payload)
                archive.addfile(member, io.BytesIO(payload))
            data = output.getvalue()
            self.assertLessEqual(len(data), kwargs["limit"])
            destination.write_bytes(data)
            transfers.append((name, len(payload)))
            return 0

        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            root = Path(temporary)
            stack.enter_context(patch.object(jobs, "JOBS", root))
            stack.enter_context(patch.object(jobs, "command", side_effect=worker_command))
            stack.enter_context(patch.object(artifacts, "command", side_effect=transfer))
            stack.enter_context(patch.object(jobs.runtime, "create", return_value="owned-fixture"))
            stack.enter_context(patch.object(jobs.runtime, "inspect", return_value={}))
            cleanup = stack.enter_context(patch.object(jobs.runtime, "remove_owned"))
            result = jobs._execute("a" * 40, mode, 120, 1, "sha256:" + "b" * 64,
                                   {"infrastructure_sha256": "c" * 64})
            directory = root / result["job_id"]
            self.assertEqual(list(directory.glob("*.transfer")), [])
            self.assertEqual(cleanup.call_args_list[-1].args[1], "boot")
            self.assertEqual(result["cleanup_errors"], [])
            captured = {entry["path"]: entry["bytes"] for entry in result["artifacts"]}
        return result, captured, transfers

    def test_recovery_summaries_at_128_kib_are_exported_and_result_is_interpreted(self):
        sizes = {"result.json": 128 * 1024, "recovery.json": 128 * 1024}
        result, captured, _ = self.execute("recovery-test", sizes)

        self.assertEqual(result["status"], "success")
        self.assertEqual(result["guest_result"], {"outcome": "success"})
        for name, size in sizes.items():
            self.assertEqual(captured[name], size)

    def test_each_oversize_recovery_summary_fails_collection_and_preserves_cleanup(self):
        for name in ("result.json", "recovery.json"):
            with self.subTest(name=name):
                result, captured, _ = self.execute("recovery-test", {name: 128 * 1024 + 1})
                self.assertEqual(result["status"], "executor_error")
                self.assertIn("artifact size limit exceeded: " + name, result["error"])
                self.assertIn("131073 bytes; maximum 131072", result["error"])
                self.assertNotIn(name, captured)
                self.assertNotIn("guest_result", result)

    def test_other_modes_keep_the_64_kib_result_limit(self):
        for mode in ("ok", "terminal-test", "block-user"):
            with self.subTest(mode=mode):
                result, captured, transfers = self.execute(mode, {"result.json": 65536})
                self.assertEqual(result["status"], "success")
                self.assertEqual(captured["result.json"], 65536)
                self.assertNotIn("recovery.json", dict(transfers))
                result, captured, _ = self.execute(mode, {"result.json": 65537})
                self.assertEqual(result["status"], "executor_error")
                self.assertIn("maximum 65536", result["error"])
                self.assertNotIn("result.json", captured)
