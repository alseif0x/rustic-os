# SPDX-License-Identifier: Apache-2.0
"""Host measurement admission and regression decisions; not guest evidence."""
import copy
import math
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from measurement.model import METRICS, admitted, compare, distribution, fingerprint
from measurement.provenance import load_bytes


def fixture(value=.01):
    config = {"host": "fixture", "workload": "fixture"}
    return {"schema": 1, "configuration": config, "configuration_id": fingerprint(config),
            "requested_samples": 5,
            "source": {"revision": "0" * 40, "build_id": "1" * 16, "worktree_status": "",
                       "kernel_sha256": "2" * 64, "image_sha256": {"ok": "3" * 64, "terminal": "4" * 64}},
            "injected_delay_ticks": 0, "status": "success",
            "samples": [{"warmup": i == 0, "status": "success", "vm_boots_started": 2,
                         "configuration_before": fingerprint(config), "configuration_after": fingerprint(config),
                         "disk_sha256": "5" * 64,
                         "artifacts": {name: "5" * 64 for name in
                                       ("probe/serial.log", "probe/qemu.log", "probe/result.json",
                                        "serial.log", "qemu.log", "files.bin")},
                         "metrics": {k: value if k.endswith("_seconds") else 100 for k in METRICS}}
                        for i in range(6)]}


class MeasurementTests(unittest.TestCase):
    def test_warmup_is_excluded_and_dispersion_is_explicit(self):
        report = fixture()
        report["samples"][0]["metrics"]["read_seconds"] = 999
        self.assertEqual(admitted(report)["read_seconds"]["max"], .01)
        self.assertEqual(distribution([1, 2, 3, 4, 5]), {"count": 5, "min": 1, "median": 3, "max": 5, "mad": 1})

    def test_held_out_control_with_small_noise_passes(self):
        self.assertEqual(compare(fixture(.01), fixture(.02))["status"], "pass")

    def test_sustained_read_delay_is_a_regression(self):
        candidate = fixture()
        for sample in candidate["samples"]:
            sample["metrics"]["read_seconds"] = 1
        result = compare(fixture(), candidate)
        self.assertEqual(result["status"], "regression")
        self.assertTrue(result["metrics"]["read_seconds"]["regression"])

    def test_counter_regression_cannot_hide_in_the_median(self):
        candidate = fixture()
        candidate["samples"][-1]["metrics"]["manager_metadata_bytes"] += 8
        self.assertEqual(compare(fixture(), candidate)["status"], "regression")

    def test_configuration_change_is_not_a_performance_regression(self):
        candidate = fixture()
        candidate["configuration"]["host"] = "different-host"
        candidate["configuration_id"] = fingerprint(candidate["configuration"])
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_configuration_digest_is_rechecked(self):
        candidate = fixture()
        candidate["configuration"]["host"] = "tampered"
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_failed_attempts_and_failed_warmups_are_not_dropped(self):
        for position in (0, 3):
            for status in ("timeout", "invalid"):
                with self.subTest(position=position, status=status):
                    candidate = fixture()
                    candidate["samples"][position]["status"] = status
                    self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_missing_samples_and_different_counts_are_rejected(self):
        candidate = fixture()
        candidate["samples"].pop()
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")
        candidate = fixture()
        candidate["samples"].append(copy.deepcopy(candidate["samples"][-1]))
        candidate["requested_samples"] = 6
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_injected_runs_cannot_be_baselines(self):
        baseline = fixture()
        baseline["injected_delay_ticks"] = 100
        self.assertEqual(compare(baseline, fixture())["status"], "incomparable")

    def test_invalid_numbers_and_metric_sets_are_rejected(self):
        for value in (True, math.nan, math.inf, -1, 10 ** 400):
            with self.subTest(value=str(value)[:20]):
                candidate = fixture()
                candidate["samples"][1]["metrics"]["read_seconds"] = value
                self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")
        candidate = fixture()
        del candidate["samples"][1]["metrics"]["read_seconds"]
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_failed_collection_status_cannot_be_overridden_by_samples(self):
        candidate = fixture()
        candidate["status"] = "incomplete"
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_configuration_drift_within_a_sample_is_rejected(self):
        candidate = fixture()
        candidate["samples"][2]["configuration_after"] = "changed"
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_integer_warmup_flag_is_rejected(self):
        candidate = fixture()
        candidate["samples"][0]["warmup"] = 1
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_missing_source_evidence_is_rejected_on_either_side(self):
        paths = [("source",)] + [("source", key) for key in
                                  ("revision", "build_id", "worktree_status", "kernel_sha256", "image_sha256")]
        paths += [("source", "image_sha256", name) for name in ("ok", "terminal")]
        for path in paths:
            for side in (0, 1):
                with self.subTest(path=path, side=side):
                    reports = [fixture(), fixture()]
                    container = reports[side]
                    for key in path[:-1]:
                        container = container[key]
                    del container[path[-1]]
                    self.assertEqual(compare(*reports)["status"], "incomparable")

    def test_invalid_source_identifiers_and_hashes_are_rejected(self):
        fields = [("revision", 40), ("build_id", 16), ("kernel_sha256", 64),
                  ("ok", 64), ("terminal", 64)]
        for field, length in fields:
            for value in (None, True, "", "a" * (length - 1), "a" * (length + 1), "g" * length):
                with self.subTest(field=field, value=value):
                    candidate = fixture()
                    container = candidate["source"]
                    if field in ("ok", "terminal"):
                        container = container["image_sha256"]
                    container[field] = value
                    self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_source_evidence_requires_shaped_images_and_explicit_worktree_status(self):
        for field, value in (("worktree_status", None), ("image_sha256", None),
                             ("image_sha256", {"ok": "3" * 64, "terminal": "4" * 64, "unknown": "6" * 64})):
            with self.subTest(field=field, value=value):
                candidate = fixture()
                candidate["source"][field] = value
                self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")
        candidate = fixture()
        candidate["source"]["worktree_status"] = " M kernel/src/main.rs\n"
        self.assertEqual(compare(fixture(), candidate)["status"], "pass")

    def test_missing_sample_evidence_invalidates_warmups_and_measurements(self):
        paths = [("disk_sha256",), ("artifacts",)] + [("artifacts", name) for name in
                  ("probe/serial.log", "probe/qemu.log", "probe/result.json", "serial.log", "qemu.log", "files.bin")]
        for position in (0, 1):
            for path in paths:
                with self.subTest(position=position, path=path):
                    candidate = fixture()
                    container = candidate["samples"][position]
                    for key in path[:-1]:
                        container = container[key]
                    del container[path[-1]]
                    self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_invalid_sample_artifact_and_disk_hashes_are_rejected(self):
        fields = ("disk_sha256", "probe/serial.log", "probe/qemu.log", "probe/result.json",
                  "serial.log", "qemu.log", "files.bin")
        for field in fields:
            for value in (None, True, "", "a" * 63, "a" * 65, "g" * 64):
                with self.subTest(field=field, value=value):
                    candidate = fixture()
                    sample = candidate["samples"][1]
                    container = sample if field == "disk_sha256" else sample["artifacts"]
                    container[field] = value
                    self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_disk_hash_must_match_the_preserved_artifact(self):
        candidate = fixture()
        candidate["samples"][1]["disk_sha256"] = "6" * 64
        self.assertEqual(compare(fixture(), candidate)["status"], "incomparable")

    def test_additional_portable_artifact_evidence_is_allowed(self):
        candidate = fixture()
        candidate["samples"][1]["artifacts"]["extra-diagnostic.log"] = "6" * 64
        self.assertEqual(compare(fixture(), candidate)["status"], "pass")


class ElfReservationTests(unittest.TestCase):
    def test_load_pages_are_counted_once_even_with_overlapping_segments(self):
        data = bytearray(64 + 112)
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<Q", data, 32, 64)
        struct.pack_into("<HH", data, 54, 56, 2)
        for offset, address, size in ((64, 4096, 5000), (120, 8192, 4096)):
            struct.pack_into("<IIQQQQQQ", data, offset, 1, 0, 0, address, 0, 1, size, 4096)
        with tempfile.TemporaryDirectory() as directory:
            kernel = Path(directory) / "kernel.elf"
            kernel.write_bytes(data)
            self.assertEqual(load_bytes(kernel), 8192)

    def test_truncated_program_headers_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            kernel = Path(directory) / "kernel.elf"
            kernel.write_bytes(b"\x7fELF\x02\x01" + bytes(58))
            with self.assertRaises(ValueError):
                load_bytes(kernel)


if __name__ == "__main__":
    unittest.main()
