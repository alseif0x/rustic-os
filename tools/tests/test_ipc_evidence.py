# SPDX-License-Identifier: Apache-2.0
import sys
from pathlib import Path
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.ipc_evidence import REJECTIONS, verified
from boot_support.scenarios import records


def evidence():
    lines = ["RUSTIC IPC verified=1 ring=3 exchanges=16 rejected=22 wait=1 cancel=1 close=1 death=1 transfer=1 attenuation=1 stale=1 cross_page=1 atomic_copy=1 version=1 channels=0 handles=0 free_before=90 free_after=90"]
    lines += [f"RUSTIC IPC_REJECT case={name} code={(1 << 64) - 1 - code:#x} preserved=1" for name, code in REJECTIONS.items()]
    return "\n".join(lines)


class IpcEvidence(unittest.TestCase):
    def test_requires_actual_user_calls_and_complete_resource_recovery(self):
        good = evidence()
        self.assertTrue(verified(good, records))
        for before, after in [("ring=3", "ring=0"), ("channels=0", "channels=1"), ("handles=0", "handles=2"),
                              ("free_after=90", "free_after=89"), ("atomic_copy=1", "atomic_copy=0"),
                              ("preserved=1", "preserved=0"), ("0xfffffffffffffffb", "0xfffffffffffffffd")]:
            self.assertFalse(verified(good.replace(before, after, 1), records), before)

    def test_missing_duplicate_or_partial_results_cannot_pass(self):
        good = evidence()
        lines = good.splitlines()
        self.assertFalse(verified("\n".join(lines[:-1]), records))
        self.assertFalse(verified(good + "\n" + lines[-1], records))
        self.assertFalse(verified(good + "\n" + lines[0], records))
        self.assertFalse(verified("RUSTIC IPC verified=1", records))
