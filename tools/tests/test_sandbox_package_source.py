# SPDX-License-Identifier: Apache-2.0
import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
preparation = importlib.import_module("sandbox_support.prepare")


class PackageSource(unittest.TestCase):
    def test_unknown_source_is_rejected_before_any_preparation(self):
        with patch.object(preparation.tempfile, "TemporaryDirectory", side_effect=AssertionError("must not prepare")):
            with self.assertRaises(ValueError):
                preparation.prepare("https://unreviewed.invalid")

    def test_source_is_fingerprinted_and_recorded_without_changing_deadlines(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE",
                     "tools/environment.py", "tools/environment.toml", "tools/application.py",
                     "tools/contracts/__init__.py", "tools/contracts/read_vectors.py",
                     "contracts/services/v1/fixtures/read-ranges.json", "contracts/services/v1/fixtures/replace-cases.json",
                     "tools/sandbox_support/container/Dockerfile"]
            for name in files:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("fixture\n")
            for name in ("kernel", "crates", "apps", "licenses", ".cargo", "tools/xtask", "tools/boot_support", "tools/terminal_support"):
                (root / name).mkdir(parents=True, exist_ok=True)
            state = root / ".cache/sandbox-image.json"
            results = []
            with patch.object(preparation, "ROOT", root), patch.object(preparation, "STATE", state), \
                 patch.object(preparation.subprocess, "run") as build, \
                 patch.object(preparation.subprocess, "check_output", return_value="sha256:" + "1" * 64):
                for source in ["default", "github"]:
                    result = preparation.prepare(source)
                    results.append(result)
                    self.assertEqual(json.loads(state.read_text()), result)
                    self.assertEqual(result["package_source"], source)
                    self.assertEqual(build.call_args.kwargs["timeout"], 900)
                    args = build.call_args.args[0]
                    self.assertIn("BASE=" + preparation.BASE, args)
                    self.assertIn("UBUNTU_MIRROR=" + (result["ubuntu_mirror"] or ""), args)
            self.assertNotEqual(results[0]["infrastructure_sha256"], results[1]["infrastructure_sha256"])
            self.assertIsNone(results[0]["ubuntu_mirror"])
            self.assertEqual(results[1]["ubuntu_mirror"], "http://azure.archive.ubuntu.com/ubuntu")

    def test_candidate_run_cannot_select_a_package_source(self):
        script = Path(__file__).resolve().parents[1] / "sandbox.py"
        result = subprocess.run([sys.executable, str(script), "run", "--revision", "a" * 40,
                                 "--package-source", "github"], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn("unrecognized arguments", result.stderr)
