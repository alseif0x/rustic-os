# SPDX-License-Identifier: Apache-2.0
"""Evidence survives cleanup without converting a failed native mission into success."""
import contextlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.connection import Connection
from terminal_support.failure import preserve_failure


class TerminalFailureTests(unittest.TestCase):
    def test_recovery_verify_stops_vms_then_captures_oracle_failure_before_cleanup(self):
        from terminal_support import recovery_acceptance

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "evidence"
            metadata = {"kernel_sha256": "a", "build_id": "b"}
            (root / "image.json").write_text(json.dumps(metadata))
            events, paths = [], []
            prefix, tail = b"x" * (174 * 512), b"z" * 512
            original = AssertionError("independent oracle rejected persisted content")

            @contextlib.contextmanager
            def owned_disk(path, initialize, upgrade_recovery):
                path.write_bytes(prefix + tail)
                paths.append(path)
                try:
                    yield path
                finally:
                    events.append("disk_cleanup")
                    path.unlink()

            @contextlib.contextmanager
            def machine(image, data, serial, log, fault):
                name = log.name.split(".", 1)[0]
                events.append(f"vm_start:{name}")
                try:
                    yield Mock(wait=Mock(return_value=33))
                finally:
                    events.append(f"vm_stop:{name}")

            def reject_snapshot(data):
                events.append("oracle_failure")
                raise original

            write_bytes = Path.write_bytes

            def observe_write(path, payload):
                if path.name == "failure-reboot.files.bin":
                    self.assertTrue(paths[0].exists())
                    events.append("disk_capture")
                return write_bytes(path, payload)

            with (
                patch.object(recovery_acceptance, "package", return_value=root / "mount.img"),
                patch.object(recovery_acceptance, "disk", owned_disk),
                patch.object(recovery_acceptance, "machine", machine),
                patch.object(recovery_acceptance, "Connection"),
                patch.object(recovery_acceptance, "exercise", return_value={}),
                patch.object(recovery_acceptance, "verify_final"),
                patch.object(recovery_acceptance, "snapshot", reject_snapshot),
                patch.object(Path, "write_bytes", observe_write),
                self.assertRaises(AssertionError) as caught,
            ):
                recovery_acceptance.verify(root / "image.img", output=output)

            self.assertIs(caught.exception, original)
            self.assertEqual(events, [
                "vm_start:initial", "vm_stop:initial", "vm_start:reboot", "vm_stop:reboot",
                "oracle_failure", "disk_capture", "disk_cleanup",
            ])
            self.assertFalse(paths[0].exists())
            self.assertEqual((output / "failure-reboot.files.bin").read_bytes(), prefix)
            self.assertEqual((output / "failure-reboot.last-sector.bin").read_bytes(), tail)
            self.assertFalse((output / "failure-initial.json").exists())

    def test_raw_corrupt_disk_is_retained_before_cleanup_and_error_reraised(self):
        with tempfile.TemporaryDirectory() as output, tempfile.TemporaryDirectory() as temporary:
            output = Path(output)
            data = Path(temporary) / "data.raw"
            prefix, tail = b"x" * (174 * 512), b"z" * 512
            data.write_bytes(prefix + tail)
            failure = AssertionError("original failed command")
            with self.assertRaises(AssertionError) as caught:
                with preserve_failure(data, output, "initial", {"kernel_sha256": "abc", "build_id": "id"}):
                    raise failure
            self.assertIs(caught.exception, failure)
            data.unlink()
            self.assertEqual((output / "failure-initial.files.bin").read_bytes(), prefix)
            self.assertEqual((output / "failure-initial.last-sector.bin").read_bytes(), tail)
            evidence = json.loads((output / "failure-initial.json").read_text())
            self.assertFalse(evidence["verified"])
            self.assertEqual(evidence["error_type"], "AssertionError")
            self.assertEqual(evidence["files.bin"]["bytes"], 174 * 512)

    def test_capture_failure_does_not_hide_original(self):
        with tempfile.TemporaryDirectory() as output:
            output = Path(output)
            failure = RuntimeError("guest failure")
            with self.assertRaises(RuntimeError) as caught:
                with preserve_failure(output / "missing", output, "initial", {"kernel_sha256": "a", "build_id": "b"}):
                    raise failure
            self.assertIs(caught.exception, failure)
            evidence = json.loads((output / "failure-initial.json").read_text())
            self.assertIn("FileNotFoundError", evidence["capture_error"])

    def test_oracle_failure_after_session_and_nested_capture(self):
        with tempfile.TemporaryDirectory() as output, tempfile.TemporaryDirectory() as temporary:
            output, temporary = Path(output), Path(temporary)
            data = temporary / "data.raw"
            data.write_bytes(bytes(175 * 512))
            metadata = {"kernel_sha256": "a", "build_id": "b"}
            with self.assertRaisesRegex(AssertionError, "oracle"):
                with preserve_failure(data, output, "reboot", metadata):
                    with preserve_failure(data, output, "initial", metadata):
                        pass  # The session/owned VM completed normally.
                    raise AssertionError("oracle rejected persisted content")
            self.assertTrue((output / "failure-reboot.files.bin").exists())
            self.assertFalse((output / "failure-initial.json").exists())
            with self.assertRaisesRegex(AssertionError, "session"):
                with preserve_failure(data, output, "outer", metadata):
                    with preserve_failure(data, output, "initial", metadata):
                        raise AssertionError("session failed")
            self.assertTrue((output / "failure-initial.json").exists())
            self.assertFalse((output / "failure-outer.json").exists())

    def test_command_timing_retains_failed_response_without_sensitive_arguments(self):
        with tempfile.TemporaryDirectory() as output:
            connection = Connection.__new__(Connection)
            connection.commands = connection.attempts = 0
            connection.started = 10
            connection.timings = Path(output) / "commands.jsonl"
            with patch.object(connection, "send"), patch.object(connection, "until", return_value="\r\nerror: Uncertain\r\n> "), patch("terminal_support.connection.time.monotonic", side_effect=[12, 12.25]):
                with self.assertRaises(AssertionError):
                    connection.command("replace private-argument", "committed id=")
            evidence = json.loads(connection.timings.read_text())
            self.assertEqual(evidence["verb"], "replace")
            self.assertNotIn("private-argument", connection.timings.read_text())
            self.assertEqual(evidence["sent_seconds"], 2)
            self.assertEqual(evidence["elapsed_seconds"], .25)
            self.assertTrue(evidence["response_received"])
            self.assertFalse(evidence["accepted"])
            self.assertEqual(connection.commands, 0)


if __name__ == "__main__":
    unittest.main()
