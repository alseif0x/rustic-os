# SPDX-License-Identifier: Apache-2.0
import copy
import sys
from pathlib import Path
import struct
import tomllib
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from application import encode
from boot_support.sdk_evidence import verified
from boot_support.scenarios import records

ROOT = Path(__file__).resolve().parents[2]


class ApplicationTests(unittest.TestCase):
    def test_manifest_wire_representation(self):
        document = tomllib.loads((ROOT / "apps/sdk-probe/app.toml").read_text())
        wire = encode(document)
        self.assertEqual(len(wire), 128)
        self.assertEqual(wire[:24], struct.pack("<8sHHIHHHH", b"RUSTAPP\0", 1, 128, 65536, 1, 0, 1, 0))
        self.assertEqual(wire[24:32], (3).to_bytes(8, "little"))
        self.assertEqual(wire[32:64], b"org.rusticos.sdk-probe".ljust(32, b"\0"))
        self.assertEqual(wire[64:96], b"sdk-probe.elf".ljust(32, b"\0"))
        self.assertEqual(wire[96:], bytes(32))

    def test_manifest_rejects_bad_contract_and_paths(self):
        original = tomllib.loads((ROOT / "apps/sdk-probe/app.toml").read_text())
        for key, value in [("schema", 2), ("schema", True), ("process_abi", 1), ("ipc_version", 2),
                           ("executable", "../app.elf"), ("identity", "x" * 32),
                           ("version", [0, -1, 0]), ("version", [True, 1, 0]),
                           ("requests", ["ipc", "ipc"]), ("requests", ["root"]),
                           ("requests", "ipc"), ("unknown", 1)]:
            bad = copy.deepcopy(original)
            bad[key] = value
            with self.assertRaises(ValueError, msg=key):
                encode(bad)

    def test_missing_or_incomplete_sdk_evidence_fails(self):
        good = "RUSTIC SDK verified=1 ring=3 applications=2 exchanges=4 admission_rejected=12 parameters_rejected=4 reports=2 reclaimed=1 free_before=90 free_after=90"
        self.assertTrue(verified(good, records))
        for before, after in [("ring=3", "ring=0"), ("applications=2", "applications=1"),
                              ("admission_rejected=12", "admission_rejected=11"),
                              ("free_after=90", "free_after=89"), ("reports=2", "reports=0")]:
            self.assertFalse(verified(good.replace(before, after), records))
        for bad in ["", "RUSTIC SDK verified=1", good + "\n" + good]:
            self.assertFalse(verified(bad, records))
