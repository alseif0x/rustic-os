# SPDX-License-Identifier: Apache-2.0
import os
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from terminal_support.machine import disk, SIZE

class TerminalDisk(unittest.TestCase):
    def test_creation_is_exclusive_and_second_open_does_not_truncate(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/"data.raw"
            with disk(path,True):
                with path.open("r+b") as f:
                    f.seek(512)
                    f.write(b"owner")
                with self.assertRaises((FileExistsError, BlockingIOError)):
                    with disk(path,True): pass
                with self.assertRaises(BlockingIOError):
                    with disk(path): pass
            with self.assertRaises(FileExistsError):
                with disk(path,True): pass
            with disk(path):
                self.assertEqual(path.stat().st_size,SIZE)
                with path.open("rb") as f:
                    f.seek(512)
                    self.assertEqual(f.read(5),b"owner")
    def test_missing_wrong_size_links_and_special_files_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(FileNotFoundError):
                with disk(root/"missing"): pass
            regular=root/"small"
            regular.write_bytes(b"preserve")
            with self.assertRaises(RuntimeError):
                with disk(regular): pass
            link=root/"link"
            link.symlink_to(regular)
            with self.assertRaises(OSError):
                with disk(link): pass
            os.link(regular,root/"hard")
            with self.assertRaises(RuntimeError):
                with disk(regular): pass
            self.assertEqual(regular.read_bytes(),b"preserve")
