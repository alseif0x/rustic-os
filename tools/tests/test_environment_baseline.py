# SPDX-License-Identifier: Apache-2.0
"""Contract tests: the reference environment is selected, never guessed."""
import contextlib
import io
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment

DOCUMENT = tomllib.loads((environment.ROOT / "tools/environment.toml").read_text())


class BaselineSelection(unittest.TestCase):
    def test_release_selects_its_own_versions_and_hashes(self):
        for release in ("24.04", "26.04"):
            config = environment.resolve(DOCUMENT, release)
            self.assertEqual(config["release"], release)
            self.assertEqual(config["packages"], DOCUMENT["baselines"][release]["packages"])
            self.assertEqual(config["firmware"], DOCUMENT["baselines"][release]["firmware"])
            self.assertEqual(config["machine"], DOCUMENT["machine"])
            self.assertNotIn("baselines", config)

    def test_reviewed_releases_differ_and_cover_the_same_tools(self):
        baselines = DOCUMENT["baselines"]
        self.assertLessEqual({"24.04", "26.04"}, set(baselines))
        first, *rest = (baselines[name] for name in sorted(baselines))
        for other in rest:
            self.assertEqual(set(first["packages"]), set(other["packages"]))
            self.assertEqual(set(first["firmware"]), set(other["firmware"]))
            self.assertNotEqual(first["packages"], other["packages"])

    def test_unreviewed_release_is_refused_with_the_reviewed_ones(self):
        with self.assertRaises(RuntimeError) as refusal:
            environment.resolve(DOCUMENT, "30.10")
        self.assertIn("30.10", str(refusal.exception))
        for release in DOCUMENT["baselines"]:
            self.assertIn(release, str(refusal.exception))

    def test_a_document_without_reviewed_baselines_is_refused(self):
        for document in ({}, {"machine": "pc-q35-8.2"}, {"baselines": {}}):
            with self.subTest(document=document):
                with self.assertRaises(RuntimeError):
                    environment.resolve(document, "24.04")


class HostRelease(unittest.TestCase):
    def read(self, text):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "os-release"
            path.write_text(text)
            return environment.host_release(path)

    def test_release_is_read_from_the_running_system(self):
        self.assertEqual(self.read('ID=ubuntu\nVERSION_ID="26.04"\nNAME="Ubuntu"\n'), "26.04")

    def test_unquoted_and_single_quoted_fields_are_read(self):
        self.assertEqual(self.read("ID=ubuntu\nVERSION_ID=24.04\n"), "24.04")
        self.assertEqual(self.read("ID=ubuntu\nVERSION_ID='26.04'\n"), "26.04")

    def test_other_distributions_are_refused(self):
        with self.assertRaises(RuntimeError):
            self.read('ID=debian\nVERSION_ID="13"\n')
        with self.assertRaises(RuntimeError):
            self.read("NAME=something\n")

    def test_a_release_without_a_version_is_refused(self):
        with self.assertRaises(RuntimeError) as refusal:
            self.read("ID=ubuntu\n")
        self.assertIn("VERSION_ID", str(refusal.exception))

    def test_an_unreadable_file_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(RuntimeError):
                environment.host_release(Path(temporary) / "absent")


class BaselineApplication(unittest.TestCase):
    """install and verify act on one selected baseline, never on host drift."""

    def verify_with(self, release, versions=None, firmware=None, machines=None):
        """Run verify against the named baseline with a controlled host."""
        config = environment.resolve(DOCUMENT, release)
        installed = {**config["packages"], **(versions or {})}
        hashes = {**config["firmware"], **(firmware or {})}
        available = f"{config['machine']}  Standard PC\n" if machines is None else machines

        def check_output(command, **options):
            if command[0] == "dpkg-query":
                return installed[command[-1]]
            if command[0] == "qemu-system-x86_64":
                return available
            raise AssertionError(command)

        with patch.object(environment, "CONFIG", config), \
                patch("environment.subprocess.check_output", side_effect=check_output), \
                patch("environment.digest", side_effect=lambda path: hashes[str(path)]):
            with contextlib.redirect_stdout(io.StringIO()):
                environment.verify()

    def test_install_installs_the_selected_baseline_pins(self):
        for release in ("24.04", "26.04"):
            with self.subTest(release=release), \
                    patch.object(environment, "CONFIG", environment.resolve(DOCUMENT, release)), \
                    patch("environment.subprocess.run") as run:
                environment.install()
            update, install = run.call_args_list
            self.assertEqual(update.args[0], ["sudo", "apt-get", "update"])
            self.assertEqual(install.args[0][:5],
                             ["sudo", "apt-get", "install", "-y", "--no-install-recommends"])
            self.assertEqual(install.args[0][5:],
                             [f"{name}={version}" for name, version in
                              DOCUMENT["baselines"][release]["packages"].items()])

    def test_verify_accepts_each_reviewed_baseline(self):
        for release in ("24.04", "26.04"):
            with self.subTest(release=release):
                self.verify_with(release)

    def test_verify_refuses_a_package_version_that_differs(self):
        for release in ("24.04", "26.04"):
            with self.subTest(release=release):
                name = sorted(DOCUMENT["baselines"][release]["packages"])[0]
                with self.assertRaises(RuntimeError) as refusal:
                    self.verify_with(release, versions={name: "0:9.9.9-replaced"})
                self.assertIn(name, str(refusal.exception))
                self.assertIn("0:9.9.9-replaced", str(refusal.exception))

    def test_verify_refuses_firmware_bytes_that_differs(self):
        name = sorted(DOCUMENT["baselines"]["26.04"]["firmware"])[0]
        with self.assertRaises(RuntimeError) as refusal:
            self.verify_with("26.04", firmware={name: "0" * 64})
        self.assertIn(name, str(refusal.exception))

    def test_verify_refuses_a_host_without_the_reference_machine(self):
        with self.assertRaises(RuntimeError) as refusal:
            self.verify_with("24.04", machines="pc-i440fx-9.0  Standard PC\n")
        self.assertIn("machine", str(refusal.exception))


if __name__ == "__main__":
    unittest.main()
