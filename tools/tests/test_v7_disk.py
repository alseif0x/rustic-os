# SPDX-License-Identifier: Apache-2.0
"""Manual disk ownership, refusal and bounded-prefix validation contracts."""
import hashlib
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import oracle7, v7_disk


def genesis(path):
    # Independent format vector: four empty roots and no records/payload.
    table = bytearray(32768)
    for index, name in enumerate((b"system", b"data", b"config", b"workspaces")):
        node = bytearray(128)
        struct.pack_into("<IIQI", node, 0, index + 1, 0, 1, 0)
        node[20:24] = bytes((2, index + 1, 0, len(name)))
        node[88:88 + len(name)] = name
        struct.pack_into("<I", node, 124, zlib.crc32(node[:124]))
        table[index * 128:(index + 1) * 128] = node
    header = bytearray(512)
    header[:8] = b"RUSTFS3\0"
    header[8:11] = bytes((7, 1, 0))
    header[12:28] = bytes(range(1, 17))
    struct.pack_into("<QQI", header, 28, 1, 1, 5)
    struct.pack_into("<III", header, 48, zlib.crc32(table),
                     zlib.crc32(bytes(16384)), zlib.crc32(bytes(2048)))
    struct.pack_into("<IIIIIIIB", header, 60, 256, 128, 16384, 131072, 8, 15, 192, 8)
    struct.pack_into("<I", header, 508, zlib.crc32(header))
    with Path(path).open("xb") as out:
        out.truncate(v7_disk.PREFIX_BYTES)
        out.seek(8 * 512)
        out.write(header)
        out.seek(10 * 512)
        out.write(table)


def provision(command, **kwargs):
    assert command[:2] == ["volume-tool", "provision7"]
    assert len(command[3]) == 32 and int(command[3], 16) != 0
    genesis(command[2])


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
