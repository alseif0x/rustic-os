# SPDX-License-Identifier: Apache-2.0
"""Failure evidence is bounded, session-scoped data across the sandbox boundary."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from sandbox_support import boot_failure
from sandbox_support.artifacts import unpack_bundle
from sandbox_support.container import export_failure


def bundle(entries):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, payload, kind in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.size = len(payload) if kind == tarfile.REGTYPE else 0
            if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                member.linkname = "../outside"
            archive.addfile(member, io.BytesIO(payload) if member.size else None)
    return output.getvalue()


class FailureExporterTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.mode = self.root / "recovery-test"
        self.mode.mkdir()
        self.base = {"image.json": b"{}", "serial.log": b"guest failure", "qemu.log": b"qemu log"}
        for name, payload in self.base.items():
            (self.mode / name).write_bytes(payload)

    def add_session(self, session):
        files = export_failure.session_files(session)
        for name in files:
            (self.mode / name).write_bytes(b"evidence for " + session.encode())
        (self.mode / f"failure-{session}.json").write_text(
            json.dumps({"session": session, "verified": False}))
        return set(files)

    def exported(self):
        output = io.BytesIO()
        export_failure.export(self.root, "recovery-test", output)
        self.assertLessEqual(len(output.getvalue()), export_failure.MAX_BUNDLE)
        with tarfile.open(fileobj=io.BytesIO(output.getvalue()), mode="r:") as archive:
            self.assertTrue(all(member.isfile() for member in archive))
            artifacts = {member.name: archive.extractfile(member).read() for member in archive}
        report = json.loads(artifacts.pop(export_failure.REPORT))
        self.assertEqual(set(report["captured"]), set(artifacts))
        return artifacts, report

    def test_exports_only_fixed_files_and_matching_failure_session(self):
        selected = self.add_session("reboot")
        (self.mode / "initial.serial.log").write_bytes(b"unselected session")
        (self.mode / "failure-unknown.json").write_bytes(b'{"verified": false}')
        (self.mode / "files.bin").write_bytes(b"unselected shared disk")
        (self.mode / "nested").mkdir()
        (self.mode / "nested" / "serial.log").write_bytes(b"not walked")

        artifacts, report = self.exported()

        self.assertEqual(set(artifacts), set(self.base) | selected)
        self.assertEqual({name: artifacts[name] for name in self.base}, self.base)
        self.assertEqual(report["status"], "captured")
        self.assertEqual(report["missing"], [])
        self.assertEqual(report["errors"], [])

    def test_invalid_metadata_is_retained_without_selecting_session_extras(self):
        invalid = [b"not JSON", b"\xff", b"[]",
                   b'{"session": "reboot", "verified": false}',
                   b'{"session": "initial", "verified": true}',
                   b'{"session": "initial", "verified": 0}',
                   b'{"session": "initial", "verified": true, "verified": false}']
        for payload in invalid:
            with self.subTest(payload=payload):
                self.add_session("initial")
                (self.mode / "failure-initial.json").write_bytes(payload)

                artifacts, report = self.exported()

                self.assertEqual(set(artifacts), set(self.base) | {"failure-initial.json"})
                self.assertEqual(artifacts["failure-initial.json"], payload)
                self.assertEqual(report["status"], "partial")
                self.assertEqual(report["missing"], [])
                self.assertEqual([entry["path"] for entry in report["errors"]],
                                 ["failure-initial.json"])

    def test_scheduling_authority_failures_keep_their_fixed_session_evidence(self):
        selected = set()
        for name in ('human_edit', 'read_write', 'inspect_all', 'scope_subject', 'revoked_cancel'):
            for ending in ('', '-reboot'):
                selected |= self.add_session('scheduling_authority_' + name + ending)
        artifacts, report = self.exported()
        self.assertEqual(set(artifacts), set(self.base) | selected)
        self.assertEqual(report['status'], 'captured')
        with self.assertRaisesRegex(ValueError, 'unknown recovery session'):
            export_failure.session_files('scheduling_authority_unreviewed')

    def test_missing_selected_extras_produce_partial_capture(self):
        selected = self.add_session("data-fault")
        missing = {"failure-data-fault.files.bin", "data-fault.qemu.log"}
        for name in missing:
            (self.mode / name).unlink()

        artifacts, report = self.exported()

        self.assertEqual(set(artifacts), set(self.base) | (selected - missing))
        self.assertEqual(report["status"], "partial")
        self.assertEqual(set(report["missing"]), missing)
        self.assertEqual(report["errors"], [])

    def test_links_and_special_files_cannot_supply_evidence(self):
        name = "serial.log"
        target = self.root / "outside"
        target.write_bytes(b"private host data")
        for kind in ("symlink", "hardlink", "fifo", "directory"):
            with self.subTest(kind=kind):
                path = self.mode / name
                path.unlink()
                if kind == "symlink":
                    path.symlink_to(target)
                elif kind == "hardlink":
                    os.link(target, path)
                elif kind == "fifo":
                    os.mkfifo(path)
                else:
                    path.mkdir()
                try:
                    artifacts, report = self.exported()
                    self.assertNotIn(name, artifacts)
                    self.assertEqual(report["status"], "partial")
                    self.assertEqual([entry["path"] for entry in report["errors"]], [name])
                    self.assertEqual(target.read_bytes(), b"private host data")
                finally:
                    path.rmdir() if kind == "directory" else path.unlink()
                    path.write_bytes(self.base[name])

    def test_oversize_source_is_rejected_and_other_evidence_survives(self):
        with (self.mode / "serial.log").open("wb") as stream:
            stream.truncate(export_failure.BASE["serial.log"] + 1)

        artifacts, report = self.exported()

        self.assertEqual(set(artifacts), {"image.json", "qemu.log"})
        self.assertEqual(report["status"], "partial")
        self.assertEqual(report["errors"][0]["path"], "serial.log")
        self.assertIn("size limit", report["errors"][0]["error"])

    def test_many_selected_files_stay_within_global_bundle_budget(self):
        for session in ("initial", "reboot", "data-fault"):
            self.add_session(session)
            for name, maximum in export_failure.session_files(session).items():
                if name != f"failure-{session}.json":
                    with (self.mode / name).open("wb") as stream:
                        stream.truncate(maximum)

        artifacts, report = self.exported()

        self.assertIn("initial.commands.jsonl", artifacts)
        self.assertEqual(report["status"], "partial")
        self.assertTrue(any("bundle budget" in entry["error"] for entry in report["errors"]))

    def test_recovery_summaries_cross_both_export_boundaries_at_the_fixed_ceiling(self):
        payload = b"{}" + b" " * (128 * 1024 - 2)
        for name in ("result.json", "recovery.json"):
            (self.mode / name).write_bytes(payload)
        output = io.BytesIO()

        export_failure.export(self.root, "recovery-test", output)
        artifacts = unpack_bundle(output.getvalue(), export_failure.allowed_files("recovery-test"),
                                  export_failure.MAX_BUNDLE)

        for name in ("result.json", "recovery.json"):
            self.assertEqual(artifacts[name], payload)
        report = json.loads(artifacts[export_failure.REPORT])
        self.assertEqual(report["status"], "captured")
        self.assertEqual(report["errors"], [])
        for name in ("result.json", "recovery.json"):
            with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, "artifact size limit"):
                oversized = bundle([(name, payload + b" ", tarfile.REGTYPE)])
                unpack_bundle(oversized, export_failure.allowed_files("recovery-test"),
                              export_failure.MAX_BUNDLE)

    def test_recovery_summary_one_byte_over_budget_is_reported_without_discarding_logs(self):
        for name in ("result.json", "recovery.json"):
            with (self.mode / name).open("wb") as stream:
                stream.truncate(128 * 1024 + 1)

        artifacts, report = self.exported()

        self.assertEqual(set(artifacts), set(self.base))
        self.assertEqual(report["status"], "partial")
        self.assertEqual({entry["path"] for entry in report["errors"]},
                         {"result.json", "recovery.json"})
        self.assertTrue(all("size limit" in entry["error"] for entry in report["errors"]))


class FailureBundleTests(unittest.TestCase):
    def test_regular_allowed_files_are_returned_as_bytes(self):
        data = bundle([("serial.log", b"failure", tarfile.REGTYPE),
                       ("image.json", b"{}", tarfile.REGTYPE)])
        self.assertEqual(unpack_bundle(data, {"serial.log": 7, "image.json": 2}, len(data)),
                         {"serial.log": b"failure", "image.json": b"{}"})

    def test_unknown_traversal_absolute_and_duplicate_members_are_rejected(self):
        for names in (["other.log"], ["../serial.log"], ["/serial.log"],
                      ["./serial.log"], ["folder/serial.log"], ["serial.log", "serial.log"]):
            with self.subTest(names=names), self.assertRaises(RuntimeError):
                data = bundle([(name, b"failure", tarfile.REGTYPE) for name in names])
                unpack_bundle(data, {"serial.log": 7}, len(data))

    def test_links_directories_and_special_members_are_rejected(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.DIRTYPE,
                     tarfile.FIFOTYPE, tarfile.CHRTYPE, tarfile.BLKTYPE):
            with self.subTest(kind=kind), self.assertRaises(RuntimeError):
                data = bundle([("serial.log", b"", kind)])
                unpack_bundle(data, {"serial.log": 7}, len(data))

    def test_member_and_archive_size_limits_are_enforced(self):
        data = bundle([("serial.log", b"failure", tarfile.REGTYPE)])
        with self.assertRaisesRegex(RuntimeError, "artifact size limit"):
            unpack_bundle(data, {"serial.log": 6}, len(data))
        with self.assertRaisesRegex(RuntimeError, "bundle size limit"):
            unpack_bundle(data, {"serial.log": 7}, len(data) - 1)

    def test_malformed_and_truncated_archives_are_rejected(self):
        complete = bundle([("serial.log", b"failure", tarfile.REGTYPE)])
        for data in (b"not a tar archive", complete[:512] + b"fail"):
            with self.subTest(length=len(data)), self.assertRaises((RuntimeError, tarfile.TarError)):
                unpack_bundle(data, {"serial.log": 7}, 65536)


class FailureCollectorTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.transfer = self.directory / "boot-failure-evidence.transfer"

    def test_one_docker_exec_writes_validated_evidence_and_removes_transfer(self):
        report = {"schema_version": 1, "mode": "recovery-test", "status": "partial",
                  "captured": ["serial.log"], "missing": ["image.json"],
                  "errors": [{"path": "qemu.log", "error": "capture rejected"}]}
        payload = bundle([("serial.log", b"original failure", tarfile.REGTYPE),
                          (export_failure.REPORT, json.dumps(report).encode(), tarfile.REGTYPE)])

        def transfer(args, destination, **kwargs):
            destination.write_bytes(payload)
            return 0

        with patch.object(boot_failure, "command", side_effect=transfer) as command:
            result = boot_failure.collect("owned-boot", "recovery-test", self.directory)

        command.assert_called_once_with(
            ["docker", "exec", "owned-boot", "python3", "-I",
             "/opt/controller/export_failure.py", "recovery-test"],
            self.transfer, timeout=30, limit=export_failure.MAX_BUNDLE)
        self.assertEqual(result, report)
        self.assertEqual((self.directory / "serial.log").read_bytes(), b"original failure")
        self.assertEqual(json.loads((self.directory / export_failure.REPORT).read_bytes()), report)
        self.assertFalse(self.transfer.exists())

    def test_nonzero_exporter_returns_failure_and_preserves_existing_logs(self):
        (self.directory / "boot.log").write_bytes(b"original worker failure")

        def fail(args, destination, **kwargs):
            destination.write_bytes(b"incomplete archive")
            return 73

        with patch.object(boot_failure, "command", side_effect=fail) as command:
            result = boot_failure.collect("owned-boot", "recovery-test", self.directory)

        command.assert_called_once()
        self.assertEqual(result["status"], "failed")
        self.assertIn("exporter exited with code 73", result["error"])
        self.assertEqual((self.directory / "boot.log").read_bytes(), b"original worker failure")
        self.assertFalse(self.transfer.exists())

    def test_transfer_errors_are_reported_and_partial_transfer_removed(self):
        errors = [OSError("transfer unavailable"),
                  subprocess.TimeoutExpired("docker", 30)]
        for error in errors:
            with self.subTest(error=type(error).__name__):
                def fail(args, destination, **kwargs):
                    destination.write_bytes(b"partial transfer")
                    raise error

                with patch.object(boot_failure, "command", side_effect=fail) as command:
                    result = boot_failure.collect("owned-boot", "recovery-test", self.directory)

                command.assert_called_once()
                self.assertEqual(result["status"], "failed")
                self.assertIn(type(error).__name__, result["error"])
                self.assertIn(str(error), result["error"])
                self.assertFalse(self.transfer.exists())

    def test_invalid_transfer_writes_no_members_and_retains_failure(self):
        invalid = [b"not a tar archive",
                   bundle([("serial.log", b"failure", tarfile.REGTYPE)]),
                   bundle([("serial.log", b"failure", tarfile.REGTYPE),
                           ("../outside", b"untrusted", tarfile.REGTYPE)])]
        for payload in invalid:
            with self.subTest(payload_bytes=len(payload)):
                def transfer(args, destination, **kwargs):
                    destination.write_bytes(payload)
                    return 0

                with patch.object(boot_failure, "command", side_effect=transfer):
                    result = boot_failure.collect("owned-boot", "recovery-test", self.directory)

                self.assertEqual(result["status"], "failed")
                self.assertTrue(result["error"])
                self.assertEqual(list(self.directory.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
