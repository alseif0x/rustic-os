# SPDX-License-Identifier: Apache-2.0
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.block_evidence import verified
from boot_support.block_runner import SECTORS, pattern, run
from boot_support.scenarios import records


def marker(phase):
    faults = phase == "faults"
    line = (f"RUSTIC BLOCK_USER verified=1 phase={phase} ring=3 applications={14 if faults else 4} "
            f"rejected={53 if faults else 0} lifecycle={6 if faults else 0} control_preemptions=4 "
            "max_bytes=512 queue_slots=2 handle_slots=4 dma_frames=3 peak_frames=79 metadata_bytes=5232 free_before=90 free_after=90")
    if not faults:
        line += (f"\nRUSTIC PUBLICATION verified=1 phase={'write' if phase == 'write' else 'replay'} "
                 f"cancelled={16 if phase == 'write' else 0} too_late=1 committed=1")
        line += (f"\nRUSTIC ADMISSION verified=1 phase={'write' if phase == 'write' else 'replay'} "
                 "admitted=1 cancelled=1 committed=1 replay_writes=0 service_control=1 fresh_authority=1")
    return line


class BlockUserEvidence(unittest.TestCase):
    def test_requires_distinct_native_boots_and_resource_recovery(self):
        good = marker("write") + "\n" + marker("read")
        self.assertTrue(verified("block-user", good, records))
        for bad in [marker("write"), marker("read"), good + "\n" + marker("read"),
                    good.replace("phase=read", "phase=write"), good.replace("ring=3", "ring=0"),
                    good.replace("free_after=90", "free_after=89"), good.replace("control_preemptions=4", "control_preemptions=0"),
                    good.replace("max_bytes=512", "max_bytes=1024"), good.replace("dma_frames=3", "dma_frames=0"),
                    good.replace("peak_frames=79", "peak_frames=0"),
                    good.replace("cancelled=16", "cancelled=15"), good.replace("too_late=1", "too_late=0"),
                    good.replace("committed=1", "committed=0"), good.replace("RUSTIC PUBLICATION ", "MISSING "),
                    good.replace("admitted=1", "admitted=0"), good.replace("replay_writes=0", "replay_writes=1"),
                    good.replace("RUSTIC ADMISSION ", "MISSING "),
                    good.replace("service_control=1", "service_control=0"),
                    good.replace("fresh_authority=1", "fresh_authority=0"),
                    good.replace(" service_control=1", ""), good.replace(" fresh_authority=1", "")]:
            self.assertFalse(verified("block-user", bad, records))

    def test_fault_evidence_cannot_omit_denials_or_lifecycle_cases(self):
        good = marker("faults")
        self.assertTrue(verified("block-user-faults", good, records))
        for key, old in [("applications", 14), ("rejected", 53), ("lifecycle", 6), ("queue_slots", 2), ("handle_slots", 4)]:
            self.assertFalse(verified("block-user-faults", good.replace(f"{key}={old}", f"{key}={old-1}"), records))
        self.assertFalse(verified("block-user-faults", good.replace("free_after=90", "free_after=invalid"), records))
        self.assertFalse(verified("block-user-faults", good + "\n" + good, records))

    def test_native_markers_cannot_replace_real_persistent_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "image.img"
            image.write_bytes(b"fixture")
            (image.parent / "image.json").write_text(json.dumps({"mode": "block-user"}))
            calls = []
            def fake_boot(image, timeout, storage, output):
                calls.append(storage)
                (output / "serial.log").write_text(marker("write" if len(calls) == 1 else "read"))
                (output / "qemu.log").write_text("")
                return {"outcome": "success", "elapsed_seconds": 0}
            with self.assertRaises(RuntimeError):
                run(image, 1, fake_boot)
            self.assertEqual(len(calls), 1)  # Host rejects missing writes before the second boot.

    def test_second_native_boot_failure_cannot_count_as_persistence(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "image.img"
            image.write_bytes(b"fixture")
            (image.parent / "image.json").write_text(json.dumps({"mode": "block-user"}))
            calls = 0
            def fake_boot(image, timeout, storage, output):
                nonlocal calls
                calls += 1
                if calls == 1:
                    disk = Path(storage[1].split("file=")[1])
                    with disk.open("r+b") as target:
                        for sector in SECTORS[1:]:
                            target.seek(sector * 512)
                            target.write(pattern(sector))
                (output / "serial.log").write_text(marker("write") if calls == 1 else "")
                (output / "qemu.log").write_text("")
                return {"outcome": "success" if calls == 1 else "timeout", "elapsed_seconds": 0}
            # This test owns second-VM failure propagation, not volume decoding.
            with patch("boot_support.block_runner.publication_evidence.inspect", return_value={}), patch("boot_support.block_runner.admission_evidence.inspect", return_value=[]):
                result = run(image, 1, fake_boot)
            self.assertEqual(calls, 2)
            self.assertEqual(result["outcome"], "unexpected")
            self.assertFalse(result["block"]["host_verified"])

    def test_old_block_patterns_do_not_substitute_for_publication_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "image.img"
            image.write_bytes(b"fixture")
            (image.parent / "image.json").write_text(json.dumps({"mode": "block-user"}))
            def fake_boot(image, timeout, storage, output):
                disk = Path(storage[1].split("file=")[1])
                with disk.open("r+b") as target:
                    for sector in SECTORS[1:]:
                        target.seek(sector * 512)
                        target.write(pattern(sector))
                (output / "serial.log").write_text(marker("write"))
                (output / "qemu.log").write_text("")
                return {"outcome": "success", "elapsed_seconds": 0}
            with self.assertRaisesRegex(AssertionError, "no committed filesystem metadata"):
                run(image, 1, fake_boot)
