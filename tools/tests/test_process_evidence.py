# SPDX-License-Identifier: Apache-2.0
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.process_evidence import FAULTS, verified
from boot_support.scenarios import records


def evidence():
    lines = ["RUSTIC PROCESS verified=1 ring=3 elf=1 preemptions=3 isolated_faults=15 repeats=16 reclaimed=1 abi=65536 free_before=123 free_after=123"]
    for name, (vector, error, address) in FAULTS.items():
        if address is None:
            address = 0xffffffff80012000
        lines.append(f"RUSTIC PROCESS_FAULT case={name} ring=3 vector={vector} error={error:#x} address={address:#x} survivor=1 reclaimed=1")
    lines.append("RUSTIC PROCESS_MEMORY slots=4 peak_frames=52 metadata_bytes=1200 entry_stack_bytes=20480 oom_cases=3")
    return "\n".join(lines)


class ProcessEvidence(unittest.TestCase):
    def test_requires_complete_user_execution_and_reclamation(self):
        good = evidence()
        self.assertTrue(verified(good, records))
        for original, replacement in [("ring=3", "ring=0"), ("preemptions=3", "preemptions=0"),
                                      ("survivor=1", "survivor=0"), ("free_after=123", "free_after=122"),
                                      ("reclaimed=1", "reclaimed=0"), ("error=0x4", "error=0x5"),
                                      ("address=0x700008", "address=0x600008"),
                                      ("0xffffffff80012000", "0x400000"), ("vector=14", "vector=8"),
                                      ("oom_cases=3", "oom_cases=0"), ("peak_frames=52", "peak_frames=0")]:
            self.assertFalse(verified(good.replace(original, replacement, 1), records), original)

    def test_missing_or_duplicate_faults_and_partial_summary_fail(self):
        good = evidence()
        lines = good.splitlines()
        self.assertFalse(verified("\n".join(lines[:-1]), records))
        self.assertFalse(verified(good + "\n" + lines[-1], records))
        self.assertFalse(verified(good + "\n" + lines[0], records))
        self.assertFalse(verified("RUSTIC PROCESS verified=1", records))
