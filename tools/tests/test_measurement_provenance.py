# SPDX-License-Identifier: Apache-2.0
"""Cgroup configuration provenance contracts using disposable procfs fixtures."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from measurement.model import fingerprint
from measurement.provenance import cgroup_configuration


class CgroupProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.proc = self.root / "proc"
        self.mount = self.root / "cgroup mount"
        self.member = "/private-user/session/workload"
        self.write(self.proc / "self/cgroup", "0::" + self.member)
        escaped_mount = str(self.mount).replace("\\", "\\134").replace(" ", "\\040")
        self.mount_line = f"30 20 0:25 / {escaped_mount} rw,nosuid - cgroup2 cgroup rw\n"
        self.write(self.proc / "self/mountinfo", self.mount_line)
        self.level(self.mount, root=True)
        self.level(self.mount / "private-user", cpu="50000 100000", memory="268435456", cpus="0-3")
        self.level(self.mount / "private-user/session", cpu="200000 100000", cpus="0-1")
        self.leaf = self.mount / self.member.lstrip("/")
        self.level(self.leaf, cpus="0-1")

    def write(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value + "\n")

    def level(self, path, *, root=False, cpu="max 100000", memory="max", cpus="0-7"):
        self.write(path / "cgroup.controllers", "memory cpuset cpu")
        self.write(path / "cpuset.cpus.effective", cpus)
        self.write(path / "cpuset.mems.effective", "0")
        if not root:
            self.write(path / "cgroup.type", "domain")
            self.write(path / "cpu.max", cpu)
            self.write(path / "memory.max", memory)

    def collect(self):
        return cgroup_configuration(self.proc)

    def test_nested_stricter_ancestors_and_effective_sets_are_preserved(self):
        result = self.collect()
        levels = result["ancestors"]
        self.assertEqual([level["depth"] for level in levels], [0, 1, 2, 3])
        self.assertEqual([level["cpu_max"] for level in levels],
                         [None, "50000 100000", "200000 100000", "max 100000"])
        self.assertEqual([level["memory_max"] for level in levels], [None, "268435456", "max", "max"])
        self.assertEqual([level["cpuset_cpus_effective"] for level in levels], ["0-7", "0-3", "0-1", "0-1"])
        self.assertEqual([level["cpuset_mems_effective"] for level in levels], ["0"] * 4)

    def test_membership_and_mount_paths_are_not_published(self):
        result = self.collect()
        encoded = json.dumps(result)
        self.assertEqual(result["membership_sha256"], fingerprint(self.member))
        self.assertNotIn("private-user", encoded)
        self.assertNotIn(str(self.root), encoded)
        self.assertNotIn("cgroup mount", encoded)

    def test_stricter_parent_change_changes_configuration_fingerprint(self):
        before = fingerprint(self.collect())
        self.write(self.mount / "private-user/memory.max", "134217728")
        self.assertNotEqual(before, fingerprint(self.collect()))

    def test_disabled_leaf_controls_preserve_limits_in_ancestors(self):
        self.write(self.leaf / "cgroup.controllers", "")
        for name in ("cpu.max", "memory.max", "cpuset.cpus.effective", "cpuset.mems.effective"):
            (self.leaf / name).unlink()
        levels = self.collect()["ancestors"]
        self.assertEqual(levels[-1]["controllers"], [])
        self.assertIsNone(levels[-1]["cpu_max"])
        self.assertEqual(levels[1]["cpu_max"], "50000 100000")

    def test_root_membership_has_no_synthetic_limits(self):
        self.write(self.proc / "self/cgroup", "0::/")
        levels = self.collect()["ancestors"]
        self.assertEqual(len(levels), 1)
        self.assertIsNone(levels[0]["cpu_max"])
        self.assertIsNone(levels[0]["memory_max"])
        self.assertEqual(levels[0]["cpuset_cpus_effective"], "0-7")

    def test_v1_hybrid_and_outside_namespace_memberships_are_rejected(self):
        for value in ("2:cpu:/scope", "0::/scope\n2:memory:/scope", "0::/../scope", "0:://scope", ""):
            with self.subTest(value=value):
                self.write(self.proc / "self/cgroup", value)
                with self.assertRaises(ValueError):
                    self.collect()

    def test_subtree_missing_and_ambiguous_mounts_are_rejected(self):
        for value in (self.mount_line.replace("0:25 / ", "0:25 /hidden-parent "),
                      "", self.mount_line + self.mount_line, "malformed"):
            with self.subTest(value=value):
                self.write(self.proc / "self/mountinfo", value)
                with self.assertRaises(ValueError):
                    self.collect()

    def test_namespaced_root_with_nonroot_interfaces_is_rejected(self):
        for name, value in (("cgroup.type", "domain"), ("cpu.max", "max 100000"), ("memory.max", "max")):
            with self.subTest(name=name):
                self.write(self.mount / name, value)
                with self.assertRaisesRegex(ValueError, "ancestor"):
                    self.collect()
                (self.mount / name).unlink()

    def test_missing_root_controllers_and_threaded_scopes_are_rejected(self):
        self.write(self.mount / "cgroup.controllers", "cpu memory")
        with self.assertRaises(ValueError):
            self.collect()
        self.write(self.mount / "cgroup.controllers", "cpu memory cpuset")
        self.write(self.leaf / "cgroup.type", "domain threaded")
        with self.assertRaisesRegex(ValueError, "threaded"):
            self.collect()

    def test_missing_or_unreadable_limit_is_not_inferred_unlimited(self):
        (self.leaf / "cpu.max").unlink()
        with self.assertRaises(ValueError):
            self.collect()
        original = Path.read_text
        def unreadable(path, *args, **kwargs):
            if path == self.mount / "private-user/memory.max":
                raise PermissionError("private-user cannot read " + str(path))
            return original(path, *args, **kwargs)
        with patch.object(Path, "read_text", unreadable):
            with self.assertRaises(ValueError) as error:
                self.collect()
        self.assertNotIn("private-user", str(error.exception))
        self.assertNotIn(str(self.root), str(error.exception))

    def test_malformed_limits_are_rejected(self):
        for name, values in (("cpu.max", ("max", "1 0", "-1 100000", "0 100000")),
                             ("memory.max", ("unknown", "-1", "1 2")),
                             ("cpuset.cpus.effective", ("", "3-1", "0-3,2", "0,,1"))):
            original = (self.leaf / name).read_text()
            for value in values:
                with self.subTest(name=name, value=value):
                    self.write(self.leaf / name, value)
                    with self.assertRaises(ValueError):
                        self.collect()
            self.write(self.leaf / name, original)

    def test_membership_drift_during_collection_is_rejected(self):
        original = Path.read_text
        reads = 0
        def moving(path, *args, **kwargs):
            nonlocal reads
            if path == self.proc / "self/cgroup":
                reads += 1
                if reads == 2:
                    return "0::/different-scope\n"
            return original(path, *args, **kwargs)
        with patch.object(Path, "read_text", moving):
            with self.assertRaisesRegex(ValueError, "changed during collection"):
                self.collect()


if __name__ == "__main__":
    unittest.main()
