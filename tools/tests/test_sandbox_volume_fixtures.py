# SPDX-License-Identifier: Apache-2.0
"""block-user provisions host-side V7 fixtures only with reviewed tooling and bounded evidence."""
from contextlib import ExitStack
import hashlib
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support import admission_evidence, publication_evidence, runner
from sandbox_support import jobs, prepare, runtime
from sandbox_support.container import worker
from terminal_support import oracle7

REGIONS = ("publication.bin", "admission-terminal.bin", "admission-pending.bin")


class ReviewedVolumeTool(unittest.TestCase):
    def setUp(self):
        self.temporary = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.work = self.temporary / "work"
        self.work.mkdir()

    def tool(self, path):
        return patch.object(worker, "VOLUME_TOOL", path)

    def test_missing_tool_is_refused_without_falling_back_to_another_path(self):
        missing = self.temporary / "absent/rustic-volume"
        environ = {"RUSTIC_VOLUME_TOOL": "/opt/reference/target/debug/rustic-volume"}
        with self.tool(missing), self.assertRaisesRegex(RuntimeError, "rerun prepare"):
            worker.volume_fixtures("block-user", environ, self.work)
        self.assertEqual(environ["RUSTIC_VOLUME_TOOL"], str(missing))
        self.assertNotIn("TMPDIR", environ)
        self.assertFalse((self.work / "tmp").exists())

    def test_link_and_non_executable_tools_are_refused(self):
        executable = self.temporary / "real-tool"
        executable.write_bytes(b"\x7fELF fixture")
        executable.chmod(0o755)
        link = self.temporary / "linked-tool"
        link.symlink_to(executable)
        plain = self.temporary / "plain-tool"
        plain.write_bytes(b"\x7fELF fixture")
        plain.chmod(0o644)
        for path in (link, plain, self.temporary):
            with self.subTest(path=path.name), self.tool(path), \
                 self.assertRaisesRegex(RuntimeError, "not an executable regular file"):
                worker.volume_fixtures("block-user", {}, self.work)

    def test_reviewed_tool_is_exposed_hashed_and_given_workspace_scratch(self):
        tool = self.temporary / "rustic-volume"
        tool.write_bytes(b"\x7fELF reviewed fixture")
        tool.chmod(0o755)
        environ = {}
        with self.tool(tool):
            provenance = worker.volume_fixtures("block-user", environ, self.work)
        self.assertEqual(environ["RUSTIC_VOLUME_TOOL"], str(tool))
        self.assertEqual(provenance, {"volume_tool_sha256": hashlib.sha256(tool.read_bytes()).hexdigest()})
        scratch = Path(environ["TMPDIR"])
        self.assertEqual(scratch.parent, self.work)
        self.assertEqual(stat.S_IMODE(scratch.stat().st_mode), 0o700)

    def test_other_modes_never_require_the_tool_or_move_scratch(self):
        missing = self.temporary / "absent/rustic-volume"
        for mode in ("ok", "block-persist", "block-user-faults", "terminal-test", "recovery-test"):
            environ = {}
            with self.subTest(mode=mode), self.tool(missing):
                self.assertEqual(worker.volume_fixtures(mode, environ, self.work), {})
                self.assertEqual(environ, {"RUSTIC_VOLUME_TOOL": str(missing)})
        self.assertFalse((self.work / "tmp").exists())

    def test_prepare_builds_the_tool_offline_from_the_reviewed_tree_at_the_exposed_path(self):
        dockerfile = (Path(prepare.__file__).parent / "container/Dockerfile").read_text().splitlines()
        reference = dockerfile.index("COPY reference /opt/reference")
        builds = [number for number, line in enumerate(dockerfile)
                  if line.startswith("RUN ") and "cargo build -p rustic-volume" in line]
        self.assertEqual(len(builds), 1)
        line = dockerfile[builds[0]]
        self.assertGreater(builds[0], reference)
        self.assertTrue(line.startswith("RUN cd /opt/reference && "))
        self.assertIn("--locked", line.split())
        self.assertIn("--offline", line.split())
        self.assertRegex(line, r"install -D -m 0755 \S+/debug/rustic-volume " + re.escape(str(worker.VOLUME_TOOL)))
        self.assertIn("tools/volume", prepare.REFERENCE_TREES)

    def test_tool_source_changes_the_infrastructure_digest(self):
        root = self.temporary / "checkout"
        files = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE",
                 "tools/environment.py", "tools/environment.toml", "tools/application.py",
                 "tools/contracts/__init__.py", "tools/contracts/read_vectors.py",
                 "contracts/services/v1/fixtures/read-ranges.json",
                 "contracts/services/v1/fixtures/replace-cases.json",
                 "tools/sandbox_support/container/Dockerfile", "tools/volume/src/provision7.rs"]
        for name in files:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("fixture\n")
        for name in prepare.REFERENCE_TREES:
            (root / name).mkdir(parents=True, exist_ok=True)
        digests = []
        with patch.object(prepare, "ROOT", root), patch.object(prepare, "STATE", root / ".cache/state.json"), \
             patch.object(prepare.subprocess, "run"), \
             patch.object(prepare.subprocess, "check_output", return_value="sha256:" + "1" * 64):
            for content in ("fixture\n", "changed provisioning\n"):
                (root / "tools/volume/src/provision7.rs").write_text(content)
                digests.append(prepare.prepare("github")["infrastructure_sha256"])
        self.assertNotEqual(digests[0], digests[1])


class BlockUserEvidence(unittest.TestCase):
    def test_region_limits_match_what_the_reviewed_runner_writes(self):
        region = oracle7.VOLUME_SECTORS * oracle7.SECTOR
        self.assertEqual(publication_evidence.SIZE * 512, region)
        self.assertEqual(admission_evidence.SIZE * 512, region)
        exports = jobs.boot_exports("block-user")
        self.assertEqual({name: exports[name] for name in REGIONS}, dict.fromkeys(REGIONS, region))
        for mode in ("block-persist", "block-user-faults", "terminal-test", "recovery-test", "ok"):
            with self.subTest(mode=mode):
                self.assertFalse(set(REGIONS) & set(jobs.boot_exports(mode)))

    def test_block_user_output_and_dense_disk_regions_fit_the_workspace(self):
        exported = sum(jobs.boot_exports("block-user").values())
        disk = len(REGIONS) * oracle7.VOLUME_SECTORS * oracle7.SECTOR
        self.assertLess(exported + disk, runtime.LIMITS["work_bytes"])
        # The same dense regions would not fit the separate /tmp tmpfs.
        self.assertGreater(disk, runtime.LIMITS["tmp_bytes"])

    def test_wall_limit_honors_the_runner_per_vm_floor(self):
        for mode, floor in runner.MODE_FLOOR.items():
            with self.subTest(mode=mode):
                boots = jobs.VM_BOOTS.get(mode, 1)
                self.assertGreaterEqual(jobs.boot_stage_seconds(mode, 45), boots * floor + 30)
                self.assertEqual(jobs.boot_stage_seconds(mode, 120), boots * 120 + 30)
        self.assertEqual(jobs.boot_stage_seconds("ok", 30), 60)
        self.assertEqual(jobs.boot_stage_seconds("block-persist", 30), 90)

    def test_job_uses_the_bounded_limits_for_its_boot_stage(self):
        timeouts, maxima = {}, {}

        def command(arguments, log, **kwargs):
            log.write_bytes(b"fixture")
            if arguments[0] == "docker":
                timeouts[arguments[7]] = kwargs["timeout"]
            return 0

        def collect(container, remote, destination, maximum):
            maxima[remote.rsplit("/", 1)[-1]] = maximum
            destination.write_bytes(b'{"outcome": "success"}')
            return {"path": destination.name}

        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            stack.enter_context(patch.object(jobs, "JOBS", Path(temporary)))
            stack.enter_context(patch.object(jobs, "command", side_effect=command))
            stack.enter_context(patch.object(jobs, "collect", side_effect=collect))
            stack.enter_context(patch.object(jobs.runtime, "create", side_effect=lambda image, job, phase, path: phase))
            stack.enter_context(patch.object(jobs.runtime, "inspect", return_value={}))
            stack.enter_context(patch.object(jobs.runtime, "remove_owned"))
            result = jobs._execute("a" * 40, "block-user", 120, 45, "sha256:" + "b" * 64,
                                   {"infrastructure_sha256": "c" * 64})
        self.assertEqual(result["status"], "success")
        self.assertEqual(timeouts, {"build": 120, "boot": 2 * runner.MODE_FLOOR["block-user"] + 30})
        for name in REGIONS:
            self.assertEqual(maxima[name], oracle7.VOLUME_SECTORS * oracle7.SECTOR)


if __name__ == "__main__":
    unittest.main()
