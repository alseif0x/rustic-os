# SPDX-License-Identifier: Apache-2.0
"""Contract tests for the independent V7 volume reader (#51).

The vectors are built here from `docs/WORKSPACE-FORMAT7.md` offsets with
`zlib.crc32`, not from Rust output; `tools/fs7_test.py` separately compares the
reader with real images and the Rust mount.
"""
import struct
import sys
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import oracle7

LINEAGE = bytes(range(1, 17))


def header(generation, sequence, epoch=1, next_id=5, lineage=LINEAGE):
    raw = bytearray(512)
    raw[:8] = oracle7.MAGIC
    raw[8], raw[9], raw[10] = oracle7.VERSION, oracle7.LAYOUT, generation
    raw[12:28] = lineage
    struct.pack_into("<QQI", raw, 28, epoch, sequence, next_id)
    struct.pack_into("<IIIIIIIB", raw, 60, 256, 128, 16384, 131072, 8, 15, 192, 8)
    struct.pack_into("<I", raw, 508, zlib.crc32(bytes(raw)))
    return bytes(raw)


def image(slot0=bytes(512), slot1=bytes(512)):
    return bytes(8 * 512) + slot0 + slot1


def node(identity=5, parent=2, version=3, name=b"file", kind=1, space=2, length=0, runs=(), crc=0):
    raw = bytearray(128)
    struct.pack_into("<IIQI", raw, 0, identity, parent, version, length)
    raw[20], raw[21], raw[22], raw[23] = kind, space, len(runs), len(name)
    for index, (start, count) in enumerate(runs):
        struct.pack_into("<II", raw, 24 + index * 8, start, count)
    raw[88:88 + len(name)] = name
    struct.pack_into("<I", raw, 120, crc)
    struct.pack_into("<I", raw, 124, zlib.crc32(bytes(raw[:124])))
    return bytes(raw)


def record(state=0, cause=0, previous=1, committed=2, admission=0, terminal=2, epoch=1, instance=1,
           length=0, runs=(), obj=6, workspace=5):
    raw = bytearray(192)
    struct.pack_into("<QIIQQQQQQQII", raw, 0, 9, workspace, obj, instance, epoch, 11,
                     previous, committed, admission, terminal, length, 0)
    raw[80], raw[81], raw[82] = state, cause, len(runs)
    for index, (start, count) in enumerate(runs):
        struct.pack_into("<II", raw, 88 + index * 8, start, count)
    struct.pack_into("<I", raw, 188, zlib.crc32(bytes(raw[:188])))
    return bytes(raw)


class GeometryTests(unittest.TestCase):
    def test_the_layout_matches_the_documented_table(self):
        self.assertEqual((oracle7.header_sector(0), oracle7.header_sector(1)), (8, 9))
        self.assertEqual((oracle7.nodes_sector(0), oracle7.map_sector(0), oracle7.receipts_sector(0)), (10, 74, 106))
        self.assertEqual((oracle7.nodes_sector(1), oracle7.map_sector(1), oracle7.receipts_sector(1)), (110, 174, 206))
        self.assertEqual(oracle7.PAYLOAD_SECTOR, 210)
        self.assertEqual(oracle7.VOLUME_SECTORS, 131_282)

    def test_only_an_exact_volume_length_is_read(self):
        for length in (len(image(header(0, 1))), (oracle7.VOLUME_SECTORS - 1) * 512,
                       (oracle7.VOLUME_SECTORS + 1) * 512):
            with self.assertRaises(oracle7.Corrupt) as refused:
                oracle7.snapshot(bytes(length))
            self.assertIn("exactly", str(refused.exception))


class HeaderSelectionTests(unittest.TestCase):
    def test_adjacent_valid_copies_select_the_higher_sequence(self):
        selected, recovered, rejected = oracle7.select_header(image(header(0, 5), header(1, 6)))
        self.assertEqual((selected["generation"], selected["sequence"], recovered, rejected), (1, 6, False, {}))

    def test_an_invalid_newest_copy_falls_back_and_reports_recovery(self):
        torn = bytearray(header(1, 6))
        torn[40] ^= 1
        selected, recovered, rejected = oracle7.select_header(image(header(0, 5), bytes(torn)))
        self.assertEqual((selected["sequence"], recovered), (5, True))
        self.assertIn("checksum", rejected[1])

    def test_fresh_genesis_with_an_unwritten_copy_is_not_a_recovery(self):
        _, recovered, _ = oracle7.select_header(image(header(0, 1)))
        self.assertFalse(recovered)
        _, recovered, _ = oracle7.select_header(image(header(0, 3, next_id=6)))
        self.assertTrue(recovered)

    def test_a_copy_in_the_wrong_physical_slot_is_not_a_candidate(self):
        with self.assertRaises(oracle7.Corrupt):
            oracle7.select_header(image(header(1, 2), bytes(512)))

    def test_impossible_valid_histories_are_refused_without_fallback(self):
        for older, newer in ((header(0, 4), header(1, 6)),
                             (header(0, 5), header(1, 6, lineage=bytes(16 * [9]))),
                             (header(0, 5, next_id=9), header(1, 6, next_id=8)),
                             (header(0, 5, epoch=3), header(1, 6, epoch=2)),
                             (header(0, 6), header(1, 6))):
            with self.assertRaises(oracle7.Corrupt):
                oracle7.select_header(image(older, newer))

    def test_header_invariants_and_features_are_enforced(self):
        for bad in (header(0, 2, epoch=3), header(0, 1, next_id=4), header(0, 1, lineage=bytes(16))):
            with self.assertRaises(oracle7.Corrupt):
                oracle7.decode_header(bad, 0)
        mask7 = bytearray(header(0, 1))
        struct.pack_into("<I", mask7, 80, 7)
        struct.pack_into("<I", mask7, 508, 0)
        struct.pack_into("<I", mask7, 508, zlib.crc32(bytes(mask7)))
        with self.assertRaises(oracle7.Corrupt):
            oracle7.decode_header(bytes(mask7), 0)


class RecordTests(unittest.TestCase):
    def test_empty_slots_are_distinct_from_records(self):
        self.assertIsNone(oracle7.decode_node(bytes(128)))
        self.assertIsNone(oracle7.decode_record(bytes(192)))

    def test_node_names_and_payload_geometry(self):
        good = oracle7.decode_node(node(length=1000, runs=((7, 1), (2, 1)), crc=1))
        self.assertEqual(good["runs"], [(7, 1), (2, 1)])
        for bad in (node(name=b".."), node(name=b"a b"), node(kind=2, length=1, runs=((0, 1),)),
                    node(length=1000, runs=((0, 1),)), node(length=1500, runs=((0, 2), (1, 1))),
                    node(length=1024, runs=((131_071, 2),)), node(space=5), node(version=0)):
            with self.assertRaises(oracle7.Corrupt):
                oracle7.decode_node(bad)

    def test_record_state_arithmetic_and_causes(self):
        self.assertEqual(oracle7.decode_record(record())["state"], "direct_committed")
        self.assertEqual(oracle7.decode_record(record(2, 0, 1, 0, 3, 4))["cause"], "unknown")
        self.assertEqual(oracle7.decode_record(record(3, 0, 1, 5, 3, 5))["state"], "admitted_committed")
        for bad in (record(0, 1), record(2, 4, 1, 0, 3, 4), record(1, 0, 1, 0, 1, 0),
                    record(3, 0, 1, 5, 3, 6), record(0, 0, 2, 2, 0, 2), record(epoch=3),
                    record(obj=4), record(obj=5), record(state=4)):
            with self.assertRaises(oracle7.Corrupt):
                oracle7.decode_record(bad)

    def test_same_object_observations_cannot_decrease(self):
        first = oracle7.decode_record(record(previous=3, committed=4, terminal=4))
        second = oracle7.decode_record(record(1, 0, 2, 0, 5, 0))
        self.assertIsNotNone(oracle7._object_history(first, second))
        later = oracle7.decode_record(record(1, 0, 4, 0, 5, 0))
        self.assertIsNone(oracle7._object_history(first, later))


if __name__ == "__main__":
    unittest.main()
