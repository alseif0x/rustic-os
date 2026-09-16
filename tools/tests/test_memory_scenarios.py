# SPDX-License-Identifier: Apache-2.0
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.scenarios import frames_verified, memory_verified, reached

MEMORY = ("RUSTIC MEMORY verified=1 page_bytes=4096 limit_bytes=2147483648 metadata_bytes=131072 "
          "managed_frames=100 table_frames=10 free_before=90 free_after=90 exhausted=90 rollback=1 "
          "zero_reuse=1 spaces=2 wx=1 aliases=1 guard=1 high_boundary_frame=262144 high_frame=0 high_frames=0")
FRAMES = "RUSTIC MEMORY_FRAMES usable_bytes=409600 managed_bytes=409600 reserved_bytes=0 allocated_frames=10 free_frames=90"


class MemoryEvidence(unittest.TestCase):
    def test_wrong_address_error_or_fixture_cannot_pass_page_fault(self):
        marker = "RUSTIC MEMORY_FAULT mode=MemoryReadOnly address=0x40000000\n"
        fault = "RUSTIC EXCEPTION build=abc vector=14 error=0x3 cr2=0x40000000 emergency=0"
        self.assertTrue(reached("memory-ro", marker + fault))
        for changed in (fault.replace("vector=14", "vector=8"), fault.replace("0x3", "0x0"),
                        fault.replace("cr2=0x40000000", "cr2=0x1234"), fault.replace("emergency=0", "emergency=1")):
            self.assertFalse(reached("memory-ro", marker + changed))
        self.assertFalse(reached("memory-nx", marker + fault))
        self.assertFalse(reached("memory-ro", fault))

    def test_success_requires_matching_accounting_and_all_assertions(self):
        self.assertTrue(memory_verified(MEMORY))
        self.assertFalse(memory_verified(MEMORY.replace("free_after=90", "free_after=89")))
        self.assertFalse(memory_verified(MEMORY.replace("aliases=1", "aliases=0")))
        self.assertFalse(memory_verified("RUSTIC MEMORY verified=1"))

    def test_metadata_cost_follows_the_address_budget(self):
        # One bit per page in each of the two bitmaps; a wrong budget or cost fails.
        self.assertTrue(memory_verified(MEMORY))
        self.assertFalse(memory_verified(MEMORY.replace("limit_bytes=2147483648", "limit_bytes=1073741824")))
        self.assertFalse(memory_verified(MEMORY.replace("metadata_bytes=131072", "metadata_bytes=65536")))

    def test_high_frames_above_the_old_limit_must_stay_consistent(self):
        high = MEMORY.replace("high_frame=0 high_frames=0", "high_frame=515867 high_frames=253724")
        self.assertTrue(memory_verified(high))
        # A claimed high frame below the named boundary is not above the old limit.
        self.assertFalse(memory_verified(high.replace("high_frame=515867", "high_frame=262143")))
        # A high frame index without at least one high frame taken is inconsistent.
        self.assertFalse(memory_verified(high.replace("high_frames=253724", "high_frames=0")))

    def test_frame_report_must_split_usable_memory_exactly(self):
        serial = MEMORY + "\n" + FRAMES
        self.assertTrue(frames_verified(serial))
        self.assertFalse(frames_verified(MEMORY + "\n" + FRAMES.replace("reserved_bytes=0", "reserved_bytes=1")))
        self.assertFalse(frames_verified(MEMORY + "\n" + FRAMES.replace("managed_bytes=409600", "managed_bytes=4096")))
        self.assertFalse(frames_verified(MEMORY + "\n" + FRAMES.replace("free_frames=90", "free_frames=89")))
        self.assertFalse(frames_verified(MEMORY))  # Missing the frame line entirely.
        self.assertFalse(frames_verified(MEMORY + "\nRUSTIC MEMORY_FRAMES usable_bytes=1"))
