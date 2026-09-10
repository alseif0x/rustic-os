# SPDX-License-Identifier: Apache-2.0
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zlib
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from terminal_support.machine import disk

class RecoveryProvision(unittest.TestCase):
    def test_fresh_volumes_have_distinct_stable_checksummed_lineage(self):
        with tempfile.TemporaryDirectory() as temporary:
            envelopes=[]
            for name in ("a","b"):
                path=Path(temporary)/name
                with disk(path,True):
                    with path.open("rb") as f:f.seek(512);first=f.read(512)
                with disk(path):
                    with path.open("rb") as f:f.seek(512);self.assertEqual(f.read(512),first)
                expected=struct.unpack_from("<I",first,24)[0]
                self.assertEqual(zlib.crc32(first[:24]+bytes(4)+first[28:]),expected)
                self.assertEqual(first[:8],b"RUSTVOL1")
                envelopes.append(first)
            self.assertNotEqual(envelopes[0][8:24],envelopes[1][8:24])
    def test_upgrade_rejects_unknown_data_without_writing(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/"data"
            with disk(path,True):
                with path.open("r+b") as f:f.seek(512);f.write(bytes(512))
            with path.open("rb") as f:before=f.read(174*512)
            with self.assertRaises(RuntimeError):
                with disk(path,upgrade_recovery=True):pass
            with path.open("rb") as f:self.assertEqual(f.read(174*512),before)
            self.assertFalse(path.with_name(path.name+".pre-recovery.bin").exists())
    def test_mutually_exclusive_modes_do_not_create_a_disk(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/"data"
            with self.assertRaises(RuntimeError):
                with disk(path,True,True):pass
            self.assertFalse(path.exists())
