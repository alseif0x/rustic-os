# SPDX-License-Identifier: Apache-2.0
"""Reject ambiguous requests and never hide uncertain cleanup."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from sandbox_support import jobs, runtime


class ControlContract(unittest.TestCase):
    def test_request_rejected_before_invoking_git(self):
        with patch.object(subprocess, "check_output") as run:
            for revision, mode, build, boot in [("HEAD", "ok", 120, 30),
                                               ("a" * 40, "shell", 120, 30),
                                               ("a" * 40, "ok", 301, 30),
                                               ("a" * 40, "ok", 120, 0)]:
                with self.subTest(mode=mode, build=build, boot=boot), self.assertRaises(ValueError):
                    jobs.execute(revision, mode, build, boot)
            run.assert_not_called()

    def test_cleanup_checks_ownership(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(runtime, "inspect", return_value={"Config": {"Labels": {"rusticos.job": "other"}}}):
                with patch.object(runtime, "command") as remove, self.assertRaises(RuntimeError):
                    runtime.remove_owned("a" * 32, "build", Path(directory))
                remove.assert_not_called()

    def test_missing_container_and_broken_daemon_are_distinct(self):
        for message in ("Error: No such object: test", "error: no such object: test"):
            with patch.object(subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", message)):
                self.assertIsNone(runtime.inspect("test"))
        with patch.object(subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", "Cannot connect to daemon")):
            with self.assertRaises(RuntimeError):
                runtime.inspect("test")
