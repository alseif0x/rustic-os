# SPDX-License-Identifier: Apache-2.0
"""Contract tests for the independent v6 volume reader (#51)."""
import sys
from pathlib import Path
import struct
import unittest
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import oracle6


class GeometryTests(unittest.TestCase):
    def test_the_generation_geometry_is_the_selected_budget(self):
        """The reader's own layout constants, which the Rust tests pin separately."""
        self.assertEqual(oracle6.NODE_BYTES, 128)
        self.assertEqual(oracle6.OBJECTS, 256)
        self.assertEqual(oracle6.NODES_SECTORS, 64)
        self.assertEqual(oracle6.MAP_SECTORS, 32)
        self.assertEqual(oracle6.RECEIPT_SECTORS, 2)
        self.assertEqual(oracle6.nodes_sector(0), 9)
        self.assertEqual(oracle6.map_sector(0), 73)
        self.assertEqual(oracle6.receipts_sector(0), 105)
        self.assertEqual(oracle6.nodes_sector(1), 107)
        self.assertEqual(oracle6.map_sector(1), 171)
        self.assertEqual(oracle6.receipts_sector(1), 203)
        self.assertEqual(oracle6.PAYLOAD_SECTOR, 205)
        self.assertEqual(oracle6.DATA_SECTORS * 512, 64 * 1024 * 1024)

    def test_an_absent_or_short_image_is_not_a_volume(self):
        for data in (bytes(1024), bytes(oracle6.PAYLOAD_SECTOR * 512)):
            with self.assertRaises(AssertionError):
                oracle6.snapshot(data)

    def test_a_checksum_with_a_flipped_body_byte_is_refused(self):
        """A header CRC covers the whole sector, so the reader must see the flip."""
        sector = bytearray(512)
        sector[:8] = oracle6.MAGIC
        sector[8] = oracle6.VERSION
        sector[9:12] = b"\0\0\x02"
        struct.pack_into("<Q", sector, 12, 3)
        struct.pack_into("<I", sector, 20, oracle6.OBJECTS)
        struct.pack_into("<I", sector, 24, oracle6.NODE_BYTES)
        struct.pack_into("<I", sector, 28, oracle6.DATA_SECTORS)
        struct.pack_into("<I", sector, 48, zlib.crc32(bytes(sector)))
        at = oracle6.HEADER_SECTOR * oracle6.SECTOR
        image = bytearray((oracle6.HEADER_SECTOR + 1) * oracle6.SECTOR)
        image[at:at + oracle6.SECTOR] = sector
        self.assertEqual(oracle6._header(bytes(image))["sequence"], 3)
        image[at + 13] ^= 1
        with self.assertRaises(AssertionError):
            oracle6._header(bytes(image))
        image[at + 13] ^= 1
        image[at + 100] ^= 1
        with self.assertRaises(AssertionError):
            oracle6._header(bytes(image))


if __name__ == "__main__":
    unittest.main()
