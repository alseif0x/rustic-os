# SPDX-License-Identifier: Apache-2.0
"""The acceptance controls must never be selected by ordinary app builds."""
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import application
from boot_support.image import source_id


class TasksBuildTests(unittest.TestCase):
    def test_only_explicit_fixture_builds_instrument_owner_processes(self):
        for name in ("shell", "supervisor", "tasks", "utility"):
            for acceptance in (False, True):
                with self.subTest(name=name, acceptance=acceptance), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    manifest = root / "apps" / name / "app.toml"
                    manifest.parent.mkdir(parents=True)
                    shutil.copyfile(application.ROOT / "apps" / name / "app.toml", manifest)
                    elf = root / "target/x86_64-unknown-none/release" / name
                    elf.parent.mkdir(parents=True)
                    elf.write_bytes(bytes(64))
                    with patch.object(application.subprocess, "run") as run:
                        application.build_one(root, {}, True, name, name + ".manifest", acceptance)
                    command = run.call_args.args[0]
                    features = command[command.index("--features") + 1]
                    expected = "native,tasks-acceptance" if acceptance and name in ("shell", "supervisor") else "native"
                    self.assertEqual(features, expected)
                    self.assertIn("--locked", command)
                    self.assertIn("--offline", command)

    def test_default_pipeline_does_not_enable_fixture(self):
        with patch.object(application, "build_one") as build:
            application.build()
        for call in build.call_args_list:
            self.assertFalse(call.args[5] if len(call.args) > 5 else False)

    def test_serial_build_identity_distinguishes_acceptance_profile(self):
        self.assertNotEqual(source_id(), source_id(tasks_acceptance=True))
