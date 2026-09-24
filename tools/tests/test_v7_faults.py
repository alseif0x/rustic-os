# SPDX-License-Identifier: Apache-2.0
"""V7 fault plan, blkdebug rules, probe/mem decoding, static bound and generation classification."""
import hashlib
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.recovery_faults import CUTS, configuration, rules
from terminal_support.v7_faults import (adopts_new_generation, classify_maintenance, classify_write, decode_mem,
                                        decode_probe, expected_probes, maintenance_cuts, payload_reached,
                                        planned_sectors, publication_events, static_pages, write_cuts,
                                        write_events)
from terminal_support.oracle7 import PAYLOAD_SECTOR
from terminal_support.v7_write import SHELL_SUBJECT, pattern


LINEAGE = "ab" * 16
WORKSPACE = f"ws_{LINEAGE}_00000005"
RESOURCE = f"rs_{LINEAGE}_00000005_00000008"


class Plan(unittest.TestCase):
    def test_an_8_kib_write_is_sixteen_payload_writes_then_the_publication(self):
        events = write_events(8192)
        self.assertEqual(len(events), 16 + 100 + 3)
        self.assertEqual(events[:116], ["write_aio"] * 116)
        self.assertEqual(events[116:], ["flush_to_disk", "write_aio", "flush_to_disk"])
        self.assertEqual(len(write_events(8193)), 17 + 103)

    def test_every_named_cut_hits_the_boundary_it_names(self):
        events = write_events(8192)
        cuts = write_cuts(8192)
        self.assertEqual([cuts[name] for name in ("payload_first", "payload_second", "payload_last", "nodes_first",
                                                  "nodes_last", "map_first", "map_last", "receipts_first",
                                                  "receipts_last", "metadata_flush", "header", "final_flush")],
                         [0, 1, 15, 16, 79, 80, 111, 112, 115, 116, 117, 118])
        self.assertEqual(events[cuts["metadata_flush"]], "flush_to_disk")
        self.assertEqual(events[cuts["header"]], "write_aio")
        self.assertEqual(maintenance_cuts(), {"nodes_first": 0, "nodes_last": 63, "map_first": 64, "map_last": 95,
                                              "receipts_first": 96, "receipts_last": 99, "metadata_flush": 100,
                                              "header": 101, "final_flush": 102})
        with self.assertRaises(ValueError):
            write_cuts(1024)

    def test_only_a_cut_after_the_header_write_adopts_the_new_generation(self):
        for events in (write_events(8192), publication_events()):
            adopted = [cut for cut in range(len(events)) if adopts_new_generation(events, cut)]
            self.assertEqual(adopted, [len(events) - 1])


V5_FINAL_FLUSH = """\
[set-state]
event = "write_aio"
state = "1"
new_state = "2"

[set-state]
event = "write_aio"
state = "2"
new_state = "3"

[set-state]
event = "flush_to_disk"
state = "3"
new_state = "4"

[set-state]
event = "write_aio"
state = "4"
new_state = "5"

[set-state]
event = "write_aio"
state = "5"
new_state = "6"

[set-state]
event = "write_aio"
state = "6"
new_state = "7"

[set-state]
event = "write_aio"
state = "7"
new_state = "8"

[set-state]
event = "write_aio"
state = "8"
new_state = "9"

[set-state]
event = "write_aio"
state = "9"
new_state = "10"

[set-state]
event = "write_aio"
state = "10"
new_state = "11"

[set-state]
event = "write_aio"
state = "11"
new_state = "12"

[set-state]
event = "write_aio"
state = "12"
new_state = "13"

[set-state]
event = "write_aio"
state = "13"
new_state = "14"

[set-state]
event = "write_aio"
state = "14"
new_state = "15"

[set-state]
event = "flush_to_disk"
state = "15"
new_state = "16"

[set-state]
event = "write_aio"
state = "16"
new_state = "17"

[inject-error]
event = "flush_to_disk"
state = "17"
errno = "5"
once = "on"
"""


class Rules(unittest.TestCase):
    def test_the_chain_advances_once_per_expected_event_and_arms_one_error(self):
        text = rules(["write_aio", "flush_to_disk", "write_aio"], 2)
        self.assertEqual(text.count("[set-state]"), 2)
        self.assertEqual(text.count("[inject-error]"), 1)
        self.assertIn('event = "flush_to_disk"\nstate = "2"\nnew_state = "3"', text)
        self.assertTrue(text.endswith('[inject-error]\nevent = "write_aio"\nstate = "3"\nerrno = "5"\nonce = "on"\n'))

    def test_the_v5_cuts_keep_their_recorded_rules(self):
        # Golden text of the v5 replacement's rules before the generalization.
        self.assertEqual(configuration(CUTS["data"]),
                         '[inject-error]\nevent = "write_aio"\nstate = "1"\nerrno = "5"\nonce = "on"\n')
        self.assertEqual(configuration(CUTS["final_flush"]), V5_FINAL_FLUSH)

    def test_unknown_events_and_out_of_range_cuts_are_refused(self):
        for events, cut in ((["read_aio"], 0), ([], 0), (["write_aio"], 1), (["write_aio"], -1), (["write_aio"], True)):
            with self.assertRaises(ValueError):
                rules(events, cut)


def receipt_text(size=524288, seed=11):
    digest = hashlib.sha256(pattern(seed, size)).hexdigest()
    return (f"operation-v1 id=op_{LINEAGE}_0000000000000009 service_instance=si_{LINEAGE}_0000000000000009 "
            "state=succeeded effect=committed cancel_requested=false\r\n"
            f"receipt workspace={WORKSPACE} resource={RESOURCE} previous_version=v_0000000000000008 "
            f"version=v_0000000000000009 size={size} epoch=e_0000000000000001 key=k_0000000000000400 "
            f"sha256={digest}\r\nwrite-v7 size={size} ticks=700\r\n")


PROBE = ("probe-v7 every=64 probes=206 max=2 p50=0 total=9 free_min=51000 free_max=51000 "
         "heap_min=0 heap_max=0\r\n")


class Decoding(unittest.TestCase):
    def test_mem_decodes_every_field(self):
        text = "mem\r\nticks=12 free_frames=51000 process_slots=8 processes=3 channels=4 pending_io=0 heap_pages=0\r\n> "
        self.assertEqual(decode_mem(text), {"ticks": 12, "free_frames": 51000, "process_slots": 8, "processes": 3,
                                            "channels": 4, "pending_io": 0, "heap_pages": 0})
        with self.assertRaises(ValueError):
            decode_mem("mem\r\nerror: Busy\r\n> ")

    def test_probe_decodes_the_receipt_and_the_samples(self):
        result = decode_probe(receipt_text() + PROBE + "> ")
        self.assertEqual(result["size"], 524288)
        self.assertEqual(result["probe"], {"every": 64, "probes": 206, "max": 2, "p50": 0, "total": 9,
                                           "free_min": 51000, "free_max": 51000, "heap_min": 0, "heap_max": 0})
        self.assertEqual(decode_probe("error: Full\r\n> "), {"error": "Full"})

    def test_a_missing_duplicate_or_inconsistent_probe_line_is_rejected(self):
        for text in (receipt_text(), receipt_text() + PROBE + PROBE,
                     receipt_text() + PROBE.replace("p50=0", "p50=3"),
                     receipt_text() + PROBE.replace("total=9", "total=1"),
                     receipt_text() + PROBE.replace("free_min=51000", "free_min=52000")):
            with self.assertRaises(ValueError):
                decode_probe(text)

    def test_the_probe_samples_every_nth_chunk_and_before_the_commit(self):
        self.assertEqual(expected_probes(524288, 64), 206)
        self.assertEqual(expected_probes(40 * 128, 64), 3)
        self.assertEqual(expected_probes(40 * 129, 64), 4)


def elf(segments):
    header = bytearray(64)
    header[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<QHH", header, 32, 64, 0, 0)
    struct.pack_into("<HH", header, 54, 56, len(segments))
    table = b"".join(struct.pack("<IIQQQQQQ", kind, flags, 0, address, address, memory, memory, 4096)
                     for kind, flags, address, memory in segments)
    return bytes(header) + table


class StaticBound(unittest.TestCase):
    def test_load_pages_round_each_segment_out_to_pages_and_add_the_stack(self):
        bound = static_pages(elf([(1, 5, 0x400000, 4097), (6, 4, 0, 56), (1, 6, 0x403800, 0x900)]))
        self.assertEqual(bound["load_pages"], 2 + 2)
        self.assertEqual(bound["pages"], 4 + 16)
        self.assertEqual(bound["bytes"], 20 * 4096)
        with self.assertRaises(ValueError):
            static_pages(b"\x7fELF\x01\x01" + bytes(58))


class Payload(unittest.TestCase):
    def setUp(self):
        # Free runs: 0..3 (4), 10..19 (10), 30..39 (10), 50.. (rest). Live file
        # 4..9, a snapshot 20..29, an aliasing record (owns nothing) and 40..49.
        self.snapshot = {
            "nodes": {5: {"kind": "directory", "runs": []}, 8: {"kind": "file", "runs": [(4, 6)]},
                      9: {"kind": "file", "runs": [(40, 10)]}},
            "records": [{"aliases_live": False, "runs": [(20, 10)]}, {"aliases_live": True, "runs": [(4, 6)]}],
        }

    def test_the_plan_is_the_first_largest_free_run(self):
        self.assertEqual(planned_sectors(self.snapshot, 3 * 512), [50, 51, 52])
        self.snapshot["nodes"][9]["runs"] = [(40, 131072 - 40)]
        self.assertEqual(planned_sectors(self.snapshot, 10 * 512), list(range(10, 20)))
        with self.assertRaises(ValueError):
            planned_sectors(self.snapshot, 11 * 512)

    def image(self, sectors, content, written):
        image = bytearray((PAYLOAD_SECTOR + 64) * 512)
        for index, sector in enumerate(sectors[:written]):
            first = (PAYLOAD_SECTOR + sector) * 512
            image[first:first + 512] = content[index * 512:(index + 1) * 512].ljust(512, b"\0")
        return image

    def test_sectors_before_the_cut_are_on_the_image_and_the_cut_sector_is_not(self):
        content, sectors = pattern(5, 4 * 512 - 7), [50, 51, 52, 53]
        self.assertEqual(payload_reached(self.image(sectors, content, 3), sectors, content, 3), 3)
        self.assertEqual(payload_reached(self.image(sectors, content, 4), sectors, content, 20), 4)
        self.assertEqual(payload_reached(self.image(sectors, content, 0), sectors, content, 0), 0)
        for written, cut in ((2, 3), (4, 3), (3, 20)):
            with self.assertRaises(AssertionError):
                payload_reached(self.image(sectors, content, written), sectors, content, cut)


def view(sequence, records, files, free=100, epoch=1):
    return {"sequence": sequence, "epoch": epoch, "records": records, "files": files, "free_sectors": free}


def shell_record(key, committed, content, previous=8):
    return {"subject": SHELL_SUBJECT, "epoch": 1, "key": key, "committed": committed, "state": "direct_committed",
            "object": 8, "previous": previous, "length": len(content),
            "sha256": hashlib.sha256(content).hexdigest(), "slot": 3, "aliases_live": True, "runs": [(0, 1)]}


class Classification(unittest.TestCase):
    def setUp(self):
        self.content = pattern(5, 8192)
        self.seed = {"subject": 1, "epoch": 1, "key": 1, "committed": 3, "slot": 0, "aliases_live": True,
                     "runs": [(0, 1)]}
        self.old_file = {"id": 8, "version": 8, "size": 0, "sha256": hashlib.sha256(b"").hexdigest()}
        self.base = view(8, [self.seed], [self.old_file])

    def published(self):
        record = shell_record(0x500, 9, self.content)
        live = {"id": 8, "version": 9, "size": 8192, "sha256": record["sha256"]}
        return view(9, [self.seed, record], [live], free=84)

    def test_the_new_generation_needs_the_record_and_the_live_version(self):
        self.assertTrue(classify_write(self.base, self.published(), 8, 0x500, self.content))
        self.assertFalse(classify_write(self.base, view(8, [self.seed], [self.old_file]), 8, 0x500, self.content))

    def test_a_record_without_the_live_version_or_an_old_generation_that_changed_fails(self):
        torn = self.published()
        torn["files"] = [self.old_file]
        other_content = self.published()
        other_content["records"][1]["sha256"] = "00" * 32
        leaked = view(8, [self.seed], [self.old_file], free=99)
        skipped = view(10, [self.seed], [self.old_file])
        for observed in (torn, other_content, leaked, skipped):
            with self.assertRaises(AssertionError):
                classify_write(self.base, observed, 8, 0x500, self.content)

    def test_maintenance_is_all_records_and_the_next_epoch_or_nothing(self):
        base = view(8, [self.seed, dict(self.seed, key=2, aliases_live=False, runs=[(4, 8)])], ["f"], free=100)
        self.assertFalse(classify_maintenance(base, dict(base)))
        self.assertTrue(classify_maintenance(base, view(9, [], ["f"], free=108, epoch=2)))
        for observed in (view(9, [self.seed], ["f"], free=108, epoch=2), view(8, [self.seed], ["f"]),
                         view(9, [], ["f"], free=100, epoch=2), view(10, [], ["f"], free=108, epoch=3)):
            with self.assertRaises(AssertionError):
                classify_maintenance(base, observed)


if __name__ == "__main__":
    unittest.main()
