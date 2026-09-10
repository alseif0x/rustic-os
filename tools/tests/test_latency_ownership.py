# SPDX-License-Identifier: Apache-2.0
"""Host failures cannot leak owned processes or bless a disk with an active writer."""
import contextlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.latency import lab, runner
from terminal_support.latency.qmp import Monitor


class LatencyOwnership(unittest.TestCase):
    def test_body_failure_stops_both_processes_and_closes_monitor(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory, state = Path(temporary), {}
            backend, guest, monitor = Mock(), Mock(), Mock()
            backend.poll.return_value = guest.poll.return_value = None
            with patch.object(lab.shutil, "copyfile"), patch.object(lab, "Monitor", return_value=monitor), \
                    patch.object(lab.subprocess, "Popen", side_effect=[backend, guest]):
                with self.assertRaisesRegex(RuntimeError, "original UART failure"):
                    with lab.running(directory / "image", directory / "data", directory, directory, state):
                        raise RuntimeError("original UART failure")
            self.assertTrue(state["process_cleanup_confirmed"])
            for process in (guest, backend):
                process.terminate.assert_called_once()
                process.wait.assert_called_once_with(timeout=3)
            monitor.close.assert_called_once()

    def test_backend_startup_failure_still_stops_its_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory, state, backend = Path(temporary), {}, Mock()
            backend.poll.return_value = None
            with patch.object(lab.shutil, "copyfile"), patch.object(lab, "Monitor", side_effect=RuntimeError("QMP start")), \
                    patch.object(lab.subprocess, "Popen", return_value=backend):
                with self.assertRaisesRegex(RuntimeError, "QMP start"):
                    with lab.running(directory / "image", directory / "data", directory, directory, state):
                        self.fail("startup did not fail")
            backend.terminate.assert_called_once()
            self.assertTrue(state["process_cleanup_confirmed"])

    def test_unkillable_guest_does_not_confirm_cleanup_but_backend_is_stopped(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory, state = Path(temporary), {}
            backend, guest, monitor = Mock(), Mock(), Mock()
            backend.poll.return_value = guest.poll.return_value = None
            guest.wait.side_effect = subprocess.TimeoutExpired("owned-guest", 3)
            with patch.object(lab.shutil, "copyfile"), patch.object(lab, "Monitor", return_value=monitor), \
                    patch.object(lab.subprocess, "Popen", side_effect=[backend, guest]):
                with self.assertRaises(subprocess.TimeoutExpired):
                    with lab.running(directory / "image", directory / "data", directory, directory, state):
                        pass
            guest.kill.assert_called_once()
            backend.terminate.assert_called_once()
            monitor.close.assert_called_once()
            self.assertNotIn("process_cleanup_confirmed", state)

    def test_unquiesced_disk_is_not_captured_and_original_error_survives(self):
        @contextlib.contextmanager
        def failed(*args):
            raise RuntimeError("cleanup failed")
            yield  # Context manager must fail before yielding.
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "failed"
            with patch.object(runner, "disk", side_effect=lambda *args: contextlib.nullcontext(Path(temporary) / "data")), \
                    patch.object(runner.lab, "running", failed), patch.object(runner, "_capture") as capture:
                with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
                    runner._case(Path("image"), output, "delayed-completion", 1,
                                 {"image_sha256": "a" * 64, "kernel_sha256": "b" * 64, "build_id": "c" * 16})
            capture.assert_not_called()
            result = json.loads((output / "result.json").read_text())
            self.assertFalse(result["verified"])
            self.assertFalse(result["cleanup_confirmed"])
            self.assertIn("cannot capture a stopped disk", result["capture_error"])

    def test_capture_error_does_not_replace_the_original_uart_failure(self):
        @contextlib.contextmanager
        def stopped(image, data, temporary, output, state):
            try:
                yield Mock(), Mock(), Path("socket")
            finally:
                state["process_cleanup_confirmed"] = True
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "failed"
            with patch.object(runner, "disk", side_effect=lambda *args: contextlib.nullcontext(Path(temporary) / "data")), \
                    patch.object(runner.lab, "running", stopped), patch.object(runner, "Connection"), \
                    patch.object(runner, "_exercise", side_effect=RuntimeError("original UART failure")), \
                    patch.object(runner, "_capture", side_effect=OSError("capture failed")):
                with self.assertRaisesRegex(RuntimeError, "original UART failure"):
                    runner._case(Path("image"), output, "delayed-completion", 1,
                                 {"image_sha256": "a" * 64, "kernel_sha256": "b" * 64, "build_id": "c" * 16})
            result = json.loads((output / "result.json").read_text())
            self.assertFalse(result["verified"])
            self.assertTrue(result["cleanup_confirmed"])
            self.assertIn("capture failed", result["capture_error"])


class QmpCorrelation(unittest.TestCase):
    def call(self, replies):
        with tempfile.TemporaryDirectory() as temporary:
            monitor = Monitor.__new__(Monitor)
            monitor.socket = Mock()
            monitor.socket.recv.return_value = b"\n".join(json.dumps(reply).encode() for reply in replies) + b"\n"
            monitor.log, monitor.buffer, monitor.ticket = Path(temporary) / "qmp.jsonl", bytearray(), 0
            monitor.started, monitor.records = time.monotonic(), 0
            return monitor.call("qmp_capabilities")

    def test_events_cannot_replace_matching_command_completion(self):
        self.assertEqual(self.call([{"event": "READY"}, {"id": 1, "return": {}}]), {})
        for reply in ({"id": 2, "return": {}}, {"id": True, "return": {}}, {"id": 1},
                      {"id": 1, "error": {"class": "GenericError"}}, {"return": {}}):
            with self.subTest(reply=reply), self.assertRaises(RuntimeError):
                self.call([reply])

    def test_unending_events_and_oversized_reply_are_bounded(self):
        with self.assertRaisesRegex(RuntimeError, "event budget"):
            self.call([{"event": "READY"}] * 64)
        with self.assertRaisesRegex(RuntimeError, "size budget"):
            self.call([{"id": 1, "return": "x" * 65536}])


if __name__ == "__main__":
    unittest.main()
