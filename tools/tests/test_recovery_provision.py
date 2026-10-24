# SPDX-License-Identifier: Apache-2.0
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import oracle7, v7_disk
from terminal_support.machine import disk, SIZE
from v7_provision_fixture import provision


class RecoveryProvision(unittest.TestCase):
    @patch.object(v7_disk.subprocess, "run", side_effect=provision)
    def test_fresh_volumes_have_distinct_stable_valid_v7_lineage(self, run):
        with tempfile.TemporaryDirectory() as temporary:
            lineages = []
            for name in ("a", "b"):
                path = Path(temporary) / name
                with disk(path, True):
                    with path.open("rb") as source:
                        first = source.read(v7_disk.PREFIX_BYTES)
                with disk(path):
                    with path.open("rb") as source:
                        self.assertEqual(source.read(v7_disk.PREFIX_BYTES), first)
                state = oracle7.snapshot(first)
                self.assertEqual(state["format"], 7)
                lineages.append(state["lineage"])
            self.assertNotEqual(lineages[0], lineages[1])

    def test_legacy_recovery_upgrade_is_refused_without_writing(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "data"
            with path.open("wb") as out:
                out.write(b"RUSTFS1\0")
                out.truncate(SIZE)
            with self.assertRaisesRegex(RuntimeError, "V7 only"):
                with disk(path, upgrade_recovery=True):
                    pass
            with path.open("rb") as source:
                self.assertEqual(source.read(8), b"RUSTFS1\0")
            self.assertEqual(path.stat().st_size, SIZE)
            self.assertFalse(path.with_name(path.name + ".pre-recovery.bin").exists())
            self.assertFalse(path.with_name(path.name + ".lock").exists())

    def test_mutually_exclusive_modes_do_not_create_a_disk(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "data"
            with self.assertRaises(RuntimeError):
                with disk(path, True, True):
                    pass
            self.assertFalse(path.exists())
            self.assertFalse(path.with_name(path.name + ".lock").exists())
