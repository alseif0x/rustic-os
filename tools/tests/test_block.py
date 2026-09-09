# SPDX-License-Identifier: Apache-2.0
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.block_evidence import verified
from boot_support.block_runner import SIZE, SECTORS, inspect_disk, pattern, run
from boot_support.scenarios import records


def marker(phase):
    return f"RUSTIC BLOCK verified=1 phase={phase} sectors=8388608 sector_bytes=512 max_bytes=512 dma_frames=3 rejected=5 free_before=90 free_after=90"


class BlockEvidence(unittest.TestCase):
    def test_requires_two_distinct_phases_and_complete_recovery(self):
        good = marker("write") + "\n" + marker("read")
        self.assertTrue(verified("block-persist", good, records))
        for bad in [marker("write"), marker("read"), good + "\n" + marker("read"),
                    good.replace("phase=read", "phase=write"), good.replace("free_after=90", "free_after=89"),
                    good.replace("dma_frames=3", "dma_frames=0"), good.replace("rejected=5", "rejected=4")]:
            self.assertFalse(verified("block-persist", bad, records))

    def test_host_oracle_rejects_missing_wrong_and_truncated_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            disk = Path(temporary) / "test.raw"
            with disk.open("xb") as output:
                output.truncate(SIZE)
            self.assertEqual(inspect_disk(disk, False), bytes(2048))
            with self.assertRaises(RuntimeError):
                inspect_disk(disk, True)
            with disk.open("r+b") as output:
                for sector in SECTORS[1:]:
                    output.seek(sector * 512)
                    output.write(pattern(sector))
            self.assertEqual(len(inspect_disk(disk, True)), 2048)
            with disk.open("r+b") as output:
                output.seek(0)
                output.write(b"x")
            with self.assertRaises(RuntimeError):
                inspect_disk(disk, True)
            with disk.open("wb") as output:
                output.truncate(1024)
            with self.assertRaises(RuntimeError):
                inspect_disk(disk, False)

    def test_guest_success_text_cannot_replace_disk_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "image.img"
            image.write_bytes(b"fixture")
            (image.parent / "image.json").write_text(json.dumps({"mode": "block-persist"}))
            def fake_boot(image, timeout, storage, output):
                (output / "serial.log").write_text(marker("write"))
                (output / "qemu.log").write_text("")
                return {"outcome": "success", "elapsed_seconds": 0}
            with self.assertRaises(RuntimeError):
                run(image, 1, fake_boot)

    def test_second_boot_failure_cannot_count_as_persistence(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "image.img"
            image.write_bytes(b"fixture")
            (image.parent / "image.json").write_text(json.dumps({"mode": "block-persist"}))
            calls = 0
            def fake_boot(image, timeout, storage, output):
                nonlocal calls
                calls += 1
                disk = Path(storage[1].split("file=")[1])
                if calls == 1:
                    with disk.open("r+b") as target:
                        for sector in SECTORS[1:]:
                            target.seek(sector * 512)
                            target.write(pattern(sector))
                (output / "serial.log").write_text(marker("write") if calls == 1 else "")
                (output / "qemu.log").write_text("")
                return {"outcome": "success" if calls == 1 else "timeout", "elapsed_seconds": 0}
            result = run(image, 1, fake_boot)
            self.assertEqual(calls, 2)
            self.assertEqual(result["outcome"], "unexpected")
            self.assertFalse(result["block"]["host_verified"])
