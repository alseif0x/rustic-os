# SPDX-License-Identifier: Apache-2.0
"""A migrated image must carry exactly the seeded v5 history with an unchanged source digest."""
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_migration import admit_command, check_migrated
from terminal_support.v7_write import pattern


LINEAGE = "33" * 16
SOURCE = "ab" * 32


def seeded_record(**fields):
    base = {"state": "direct_committed", "cause": None, "subject": 2, "object": 6, "key": 0x500, "seed": 5,
            "size": 700, "previous": 9, "committed": 10, "admission": 0, "terminal": 10,
            "resource": f"rs_{LINEAGE}_00000005_00000006"}
    return {**base, **fields}


def oracle_record(seeded, **fields):
    record = {key: seeded[key] for key in ("state", "cause", "subject", "object", "key", "previous", "committed",
                                           "admission", "terminal")}
    record.update(sha256=hashlib.sha256(pattern(seeded["seed"], seeded["size"])).hexdigest(), instance=10,
                  workspace=5, epoch=1, slot=0)
    return {**record, **fields}


def fixture():
    records = [seeded_record(), seeded_record(subject=1, object=7, key=0x501, seed=6, size=300, previous=11,
                                              committed=12, terminal=12)]
    seeded = {"set": "receipts", "lineage": LINEAGE, "sequence": 12, "epoch": 1, "instance": 10,
              "workspace": {"id": 5, "text": f"ws_{LINEAGE}_00000005"}, "records": records}
    migrated = {"source_sha256_before": SOURCE, "source_sha256_after": SOURCE,
                "target": {"sequence": 12, "recovered": True}}
    snapshot = {"lineage": LINEAGE, "sequence": 12, "epoch": 1, "generation": 0, "recovered": True,
                "records": [oracle_record(record) for record in records]}
    return seeded, migrated, snapshot


class MigrationCheckTest(unittest.TestCase):
    def test_the_seeded_history_is_accepted(self):
        check_migrated(*fixture(), SOURCE)

    def test_a_changed_source_digest_is_refused(self):
        seeded, migrated, snapshot = fixture()
        migrated["source_sha256_after"] = "cd" * 32
        with self.assertRaisesRegex(AssertionError, "source digest"):
            check_migrated(seeded, migrated, snapshot, SOURCE)
        with self.assertRaisesRegex(AssertionError, "source digest"):
            check_migrated(*fixture(), "cd" * 32)

    def test_identity_and_report_disagreements_are_refused(self):
        for mutate in (lambda s, m, o: o.update(lineage="44" * 16), lambda s, m, o: o.update(epoch=2),
                       lambda s, m, o: m["target"].update(sequence=13),
                       lambda s, m, o: o.update(recovered=False), lambda s, m, o: o.update(generation=1),
                       lambda s, m, o: m["target"].update(recovered=False)):
            seeded, migrated, snapshot = fixture()
            mutate(seeded, migrated, snapshot)
            with self.assertRaises(AssertionError):
                check_migrated(seeded, migrated, snapshot, SOURCE)

    def test_any_record_difference_is_refused(self):
        changes = [{"subject": 2}, {"state": "admitted"}, {"cause": "requested"}, {"instance": 11},
                   {"workspace": 4}, {"sha256": "00" * 32}, {"key": 0x502}]
        for change in changes:
            seeded, migrated, snapshot = fixture()
            snapshot["records"][1].update(change)
            with self.assertRaises(AssertionError, msg=change):
                check_migrated(seeded, migrated, snapshot, SOURCE)
        seeded, migrated, snapshot = fixture()
        snapshot["records"].pop()
        with self.assertRaises(AssertionError):
            check_migrated(seeded, migrated, snapshot, SOURCE)

    def test_an_exact_admission_retry_names_the_seeded_request(self):
        record = seeded_record(state="admitted", key=0x510, seed=7, size=900)
        self.assertEqual(admit_command(f"ws_{LINEAGE}_00000005", record),
                         f"admit-pattern-v7 ws_{LINEAGE}_00000005 rs_{LINEAGE}_00000005_00000006 "
                         "v_0000000000000009 e_0000000000000001 k_0000000000000510 7 900")


if __name__ == "__main__":
    unittest.main()
