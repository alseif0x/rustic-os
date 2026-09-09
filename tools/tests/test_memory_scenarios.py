# SPDX-License-Identifier: Apache-2.0
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.scenarios import memory_verified, reached


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
        line = "RUSTIC MEMORY verified=1 page_bytes=4096 metadata_bytes=65536 managed_frames=100 table_frames=10 free_before=90 free_after=90 exhausted=90 rollback=1 zero_reuse=1 spaces=2 wx=1 aliases=1 guard=1"
        self.assertTrue(memory_verified(line))
        self.assertFalse(memory_verified(line.replace("free_after=90", "free_after=89")))
        self.assertFalse(memory_verified(line.replace("aliases=1", "aliases=0")))
        self.assertFalse(memory_verified("RUSTIC MEMORY verified=1"))
