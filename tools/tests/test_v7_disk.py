# SPDX-License-Identifier: Apache-2.0
"""Manual disk ownership, refusal and bounded-prefix validation contracts."""
import hashlib
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import oracle7, v7_disk
from v7_provision_fixture import provision


class ManualV7Disk(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "data.raw"

    def fresh(self):
        return v7_disk.disk(self.path, True, volume_tool="volume-tool")

    @patch.object(v7_disk.subprocess, "run", side_effect=provision)
    def test_fresh_creation_reuse_and_lock_preserve_the_volume_and_tail(self, run):
        with self.fresh():
            self.assertEqual(self.path.stat().st_size, v7_disk.SIZE)
            self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(BlockingIOError):
                with v7_disk.disk(self.path):
                    pass
            with self.path.open("r+b") as out:
                out.seek(v7_disk.SIZE - 4)
                out.write(b"tail")
        with self.path.open("rb") as source:
            before = hashlib.sha256(source.read(v7_disk.PREFIX_BYTES)).digest()
        with v7_disk.disk(self.path):
            pass
        with self.path.open("rb") as source:
            self.assertEqual(hashlib.sha256(source.read(v7_disk.PREFIX_BYTES)).digest(), before)
            source.seek(v7_disk.SIZE - 4)
            self.assertEqual(source.read(), b"tail")
        with self.assertRaises(FileExistsError):
            with self.fresh():
                pass

    def test_missing_legacy_and_wrong_size_refuse_without_writes(self):
        with self.assertRaises(FileNotFoundError):
            with v7_disk.disk(self.path):
                pass
        for marker, size in ((b"RUSTVOL1", v7_disk.SIZE),
                             (b"RUSTFS1\0", v7_disk.SIZE), (b"preserve", 100)):
            with self.path.open("wb") as out:
                out.write(marker)
                out.truncate(size)
            with self.assertRaises(RuntimeError):
                with v7_disk.disk(self.path):
                    pass
            with self.path.open("rb") as source:
                self.assertEqual(source.read(len(marker)), marker)
            self.assertEqual(self.path.stat().st_size, size)

    def test_data_links_and_special_files_are_rejected_without_blocking(self):
        original = self.path.with_name("original")
        original.write_bytes(b"preserve")
        self.path.symlink_to(original)
        with self.assertRaises(OSError):
            with v7_disk.disk(self.path):
                pass
        self.path.unlink()
        os.link(original, self.path)
        with self.assertRaises(RuntimeError):
            with v7_disk.disk(self.path):
                pass
        self.path.unlink()
        os.mkfifo(self.path)
        with self.assertRaises(RuntimeError):
            with v7_disk.disk(self.path):
                pass
        self.assertEqual(original.read_bytes(), b"preserve")

    def test_hardlinked_lock_is_refused(self):
        lock = self.path.with_name("data.raw.lock")
        lock.write_bytes(b"preserve")
        os.link(lock, self.path.with_name("other.lock"))
        with self.assertRaises(RuntimeError):
            with v7_disk.disk(self.path):
                pass
        self.assertEqual(lock.read_bytes(), b"preserve")

    @patch.object(v7_disk.subprocess, "run", side_effect=provision)
    def test_corrupt_prefix_is_refused_without_repair(self, run):
        with self.fresh():
            pass
        with self.path.open("r+b") as out:
            out.seek(10 * 512 + 88)
            out.write(b"X")
        with self.assertRaises(RuntimeError):
            with v7_disk.disk(self.path):
                pass
        with self.path.open("rb") as source:
            source.seek(10 * 512 + 88)
            self.assertEqual(source.read(1), b"X")

    def test_descriptor_identity_rejects_a_replaced_path(self):
        self.path.write_bytes(b"original")
        fd = os.open(self.path, os.O_RDONLY)
        try:
            info = os.fstat(fd)
            self.path.unlink()
            self.path.write_bytes(b"replacement")
            with self.assertRaises(RuntimeError):
                v7_disk.same_file(self.path, info)
        finally:
            os.close(fd)


if __name__ == "__main__":
    unittest.main()
