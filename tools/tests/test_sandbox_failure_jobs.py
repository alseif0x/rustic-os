# SPDX-License-Identifier: Apache-2.0
"""A failed boot remains failed while bounded evidence capture is attempted."""
from contextlib import ExitStack
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from sandbox_support import jobs


class FailedBootState(unittest.TestCase):
    def execute(self, *, failed="boot", capture=None, oom=False, inspect_failure=False,
                cleanup_failure=False):
        events = []
        inspections = 0

        def command(arguments, log, **kwargs):
            log.write_bytes(b"fixture")
            if arguments[0] == "git":
                return 0
            phase = arguments[7]
            events.append("worker-" + phase)
            return 23 if phase == failed else 0

        def collect(container, remote, destination, maximum):
            destination.write_bytes(b"bounded build artifact")
            return {"path": destination.name}

        def failure_collect(container, mode, directory):
            events.append("capture")
            return capture or {"status": "partial", "missing": ["initial.commands.jsonl"]}

        def inspect(container):
            nonlocal inspections
            inspections += 1
            if inspect_failure and inspections == 3:
                raise RuntimeError("daemon inspect unavailable")
            return {"State": {"OOMKilled": oom and inspections == 3}}

        def remove(job_id, phase, directory):
            events.append("cleanup-" + phase)
            if cleanup_failure and phase == "boot":
                raise RuntimeError("owned cleanup could not be confirmed")

        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            stack.enter_context(patch.object(jobs, "JOBS", Path(temporary)))
            stack.enter_context(patch.object(jobs, "command", side_effect=command))
            stack.enter_context(patch.object(jobs, "collect", side_effect=collect))
            stack.enter_context(patch.object(jobs, "collect_boot_failure", side_effect=failure_collect))
            stack.enter_context(patch.object(jobs.runtime, "create", side_effect=lambda image, job, phase, path: phase))
            stack.enter_context(patch.object(jobs.runtime, "inspect", side_effect=inspect))
            stack.enter_context(patch.object(jobs.runtime, "remove_owned", side_effect=remove))
            result = jobs._execute("a" * 40, "recovery-test", 120, 1,
                                   "sha256:" + "b" * 64, {"infrastructure_sha256": "c" * 64})
        return result, events

    def test_nonzero_boot_collects_before_cleanup_without_accepting_a_guest_result(self):
        result, events = self.execute()
        self.assertEqual(result["status"], "boot_failed")
        self.assertEqual(result["worker_exit_code"], 23)
        self.assertEqual(result["failure_evidence"]["status"], "partial")
        self.assertNotIn("guest_result", result)
        self.assertLess(events.index("capture"), events.index("cleanup-boot"))
        self.assertEqual(events.count("capture"), 1)
        self.assertEqual(result["cleanup_errors"], [])

    def test_capture_failure_does_not_replace_the_failed_execution(self):
        result, _ = self.execute(capture={"status": "failed", "error": "bounded transfer failed"})
        self.assertEqual(result["status"], "boot_failed")
        self.assertEqual(result["failure_evidence"]["error"], "bounded transfer failed")
        self.assertNotIn("error", result)

    def test_oom_and_inspection_failure_keep_worker_failure_provenance(self):
        result, _ = self.execute(oom=True)
        self.assertEqual(result["status"], "resource_limit")
        self.assertEqual(result["worker_exit_code"], 23)
        result, events = self.execute(inspect_failure=True)
        self.assertEqual(result["status"], "boot_failed")
        self.assertIn("daemon inspect unavailable", result["worker_inspect_error"])
        self.assertIn("capture", events)

    def test_cleanup_failure_remains_visible_after_capture(self):
        result, _ = self.execute(cleanup_failure=True)
        self.assertEqual(result["status"], "cleanup_failed")
        self.assertEqual(result["worker_exit_code"], 23)
        self.assertEqual(result["failure_evidence"]["status"], "partial")
        self.assertEqual(len(result["cleanup_errors"]), 1)

    def test_build_failure_never_runs_or_exports_boot_work(self):
        result, events = self.execute(failed="build")
        self.assertEqual(result["status"], "build_failed")
        self.assertNotIn("capture", events)
        self.assertNotIn("worker-boot", events)
        self.assertNotIn("failure_evidence", result)
