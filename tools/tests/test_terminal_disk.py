# SPDX-License-Identifier: Apache-2.0
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import v7_disk
from terminal_support.machine import disk, SIZE
from v7_provision_fixture import provision


class TerminalDisk(unittest.TestCase):
    @patch.object(v7_disk.subprocess, "run", side_effect=provision)
    def test_creation_is_exclusive_and_second_open_does_not_truncate(self, run):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "data.raw"
            with disk(path, True):
                self.assertEqual(path.stat().st_size, SIZE)
                self.assertTrue(str(run.call_args.args[0][0]).endswith("target/debug/rustic-volume"))
                with path.open("r+b") as f:
                    f.seek(512)
                    f.write(b"owner")
                with self.assertRaises((FileExistsError, BlockingIOError)):
                    with disk(path, True):
                        pass
                with self.assertRaises(BlockingIOError):
                    with disk(path):
                        pass
            with self.assertRaises(FileExistsError):
                with disk(path, True):
                    pass
            with disk(path):
                self.assertEqual(path.stat().st_size, SIZE)
                with path.open("rb") as f:
                    f.seek(512)
                    self.assertEqual(f.read(5), b"owner")

    def test_missing_legacy_wrong_size_links_and_special_files_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(FileNotFoundError):
                with disk(root / "missing"):
                    pass
            regular = root / "small"
            regular.write_bytes(b"preserve")
            with self.assertRaises(RuntimeError):
                with disk(regular):
                    pass
            self.assertEqual(regular.read_bytes(), b"preserve")
            legacy = root / "legacy"
            with legacy.open("wb") as out:
                out.write(b"RUSTFS1\0")
                out.truncate(SIZE)
            with self.assertRaises(RuntimeError):
                with disk(legacy):
                    pass
            with legacy.open("rb") as source:
                self.assertEqual(source.read(8), b"RUSTFS1\0")
            link = root / "link"
            link.symlink_to(regular)
            with self.assertRaises(OSError):
                with disk(link):
                    pass
            os.link(regular, root / "hard")
            with self.assertRaises(RuntimeError):
                with disk(regular):
                    pass
            self.assertEqual(regular.read_bytes(), b"preserve")

    @patch.object(v7_disk.subprocess, "run", side_effect=provision)
    def test_corrupt_v7_is_rejected_without_repair(self, run):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "data.raw"
            with disk(path, True):
                pass
            with path.open("r+b") as out:
                out.seek(10 * 512 + 88)
                out.write(b"X")
            with self.assertRaises(RuntimeError):
                with disk(path):
                    pass
            with path.open("rb") as source:
                source.seek(10 * 512 + 88)
                self.assertEqual(source.read(1), b"X")
