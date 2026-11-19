# SPDX-License-Identifier: Apache-2.0
"""Legacy RUSTFS1 fixture disk: provisioning shape and refusals.

The expectations mirror `crates/fs/src/volume.rs::initialize` and
`crates/fs/src/provision.rs::lineage`: the fresh legacy service initializes only
when its 174 reserved sectors are zero apart from one valid `RUSTVOL1` lineage
envelope in sector 1. They are written independently of the host helper.
"""
import fcntl
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.legacy_disk import disk
from terminal_support.machine import SIZE

SECTOR = 512
RESERVED = 174


def envelope_lineage(sector):
    """Return the lineage if `sector` is a valid R0 envelope, else None."""
    if sector[:8] != b"RUSTVOL1" or any(sector[28:]):
        return None
    expected = struct.unpack_from("<I", sector, 24)[0]
    if zlib.crc32(sector[:24] + bytes(4) + sector[28:]) != expected:
        return None
    lineage = sector[8:24]
    return None if lineage == bytes(16) else lineage


def legacy_v1_prefix():
    """A minimal valid unprovisioned version-1 RUSTFS1 bank pair."""
    prefix = bytearray(RESERVED * SECTOR)
    metadata = bytes(4 * SECTOR)
    for sector in (8, 13):
        header = bytearray(SECTOR)
        header[:12] = b"RUSTFS1\0\x01\0\0\x02"
        struct.pack_into("<I", header, 24, zlib.crc32(metadata))
        struct.pack_into("<I", header, 28, zlib.crc32(header))
        prefix[sector * SECTOR:(sector + 1) * SECTOR] = header
    return bytes(prefix)


def read_prefix(path):
    with path.open("rb") as source:
        return source.read(RESERVED * SECTOR)


class LegacyDisk(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_fresh_fixture_is_acceptable_to_the_legacy_initializer(self):
        lineages = []
        for name in ("a", "b"):
            path = self.root / name
            with disk(path, True) as data:
                self.assertEqual(data, path.resolve())
                self.assertEqual(path.stat().st_size, SIZE)
            prefix = read_prefix(path)
            self.assertEqual(prefix[:SECTOR], bytes(SECTOR))
            lineage = envelope_lineage(prefix[SECTOR:2 * SECTOR])
            self.assertIsNotNone(lineage, "fresh fixture lacks recovery lineage")
            self.assertEqual(prefix[2 * SECTOR:], bytes((RESERVED - 2) * SECTOR))
            with path.open("rb") as source:
                source.seek(-SECTOR, 2)
                self.assertEqual(source.read(SECTOR), bytes(SECTOR))
            lineages.append(lineage)
        self.assertNotEqual(lineages[0], lineages[1])

    def test_creation_is_exclusive_locked_and_reopen_preserves_bytes(self):
        path = self.root / "data.raw"
        with disk(path, True):
            with path.open("r+b") as out:
                out.seek(8 * SECTOR)
                out.write(b"guest")
            with self.assertRaises(BlockingIOError):
                with disk(path):
                    pass
        with self.assertRaises(FileExistsError):
            with disk(path, True):
                pass
        with disk(path):
            self.assertEqual(path.stat().st_size, SIZE)
            with path.open("rb") as source:
                source.seek(8 * SECTOR)
                self.assertEqual(source.read(5), b"guest")

    def test_lock_held_elsewhere_refuses_before_creating_data(self):
        path = self.root / "data.raw"
        lock = os.open(path.with_name(path.name + ".lock"), os.O_RDWR | os.O_CREAT, 0o600)
        self.addCleanup(os.close, lock)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with self.assertRaises(BlockingIOError):
            with disk(path, True):
                pass
        self.assertFalse(path.exists())

    def test_missing_wrong_size_and_linked_media_are_refused_unchanged(self):
        with self.assertRaises(FileNotFoundError):
            with disk(self.root / "missing"):
                pass
        self.assertFalse((self.root / "missing").exists())
        small = self.root / "small"
        small.write_bytes(b"preserve")
        with self.assertRaises(RuntimeError):
            with disk(small):
                pass
        self.assertEqual(small.read_bytes(), b"preserve")
        target = self.root / "target"
        with target.open("wb") as out:
            out.truncate(SIZE)
        link = self.root / "link"
        link.symlink_to(target)
        with self.assertRaises(OSError):
            with disk(link):
                pass
        os.link(target, self.root / "hard")
        with self.assertRaises(RuntimeError):
            with disk(target):
                pass
        self.assertEqual(read_prefix(target), bytes(RESERVED * SECTOR))

    def test_linked_lock_file_is_refused(self):
        path = self.root / "data.raw"
        elsewhere = self.root / "elsewhere"
        elsewhere.write_bytes(b"")
        path.with_name(path.name + ".lock").symlink_to(elsewhere)
        with self.assertRaises(OSError):
            with disk(path, True):
                pass
        self.assertFalse(path.exists())

    def test_mutually_exclusive_modes_create_nothing(self):
        path = self.root / "data.raw"
        with self.assertRaises(RuntimeError):
            with disk(path, True, True):
                pass
        self.assertFalse(path.exists())
        self.assertFalse(path.with_name(path.name + ".lock").exists())

    def test_upgrade_provisions_valid_v1_media_once_and_keeps_a_backup(self):
        path = self.root / "legacy.raw"
        original = legacy_v1_prefix()
        with path.open("wb") as out:
            out.write(original)
            out.truncate(SIZE)
        with disk(path, upgrade_recovery=True):
            pass
        backup = path.with_name(path.name + ".pre-recovery.bin")
        self.assertEqual(backup.read_bytes(), original)
        upgraded = read_prefix(path)
        self.assertIsNotNone(envelope_lineage(upgraded[SECTOR:2 * SECTOR]))
        self.assertEqual(upgraded[:SECTOR] + upgraded[2 * SECTOR:],
                         original[:SECTOR] + original[2 * SECTOR:])
        # Already provisioned media is no longer an upgrade candidate.
        with self.assertRaises(RuntimeError):
            with disk(path, upgrade_recovery=True):
                pass
        self.assertEqual(read_prefix(path), upgraded)
        self.assertEqual(backup.read_bytes(), original)

    def test_upgrade_refuses_unrecognized_media_without_writing(self):
        path = self.root / "data.raw"
        with path.open("wb") as out:
            out.truncate(SIZE)
        with self.assertRaises(RuntimeError):
            with disk(path, upgrade_recovery=True):
                pass
        self.assertEqual(read_prefix(path), bytes(RESERVED * SECTOR))
        self.assertFalse(path.with_name(path.name + ".pre-recovery.bin").exists())


if __name__ == "__main__":
    unittest.main()
