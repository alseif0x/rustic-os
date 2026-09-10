# SPDX-License-Identifier: Apache-2.0
"""Collector failures retain evidence and never become usable measurements."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from measurement.model import fingerprint
from measurement.sample import collect
from measurement.series import checked_sample


class CollectionTests(unittest.TestCase):
    def test_drift_before_collection_does_not_start_a_vm(self):
        config = {"host": {"label": "fixture"}}
        with tempfile.TemporaryDirectory() as directory, \
                patch("measurement.series.configuration", return_value={"changed": True}), \
                patch("measurement.series.collect") as start:
            output = Path(directory) / "sample"
            result = checked_sample({}, output, False, 0, config)
            start.assert_not_called()
            self.assertEqual(result["vm_boots_started"], 0)
            self.assertEqual(result["status"], "invalid")
            self.assertEqual(json.loads((output / "sample.json").read_text()), result)

    def test_unreadable_configuration_retains_a_failed_attempt(self):
        config = {"host": {"label": "fixture"}}
        with tempfile.TemporaryDirectory() as directory, \
                patch("measurement.series.configuration", side_effect=OSError("missing limit")), \
                patch("measurement.series.collect") as start:
            result = checked_sample({}, Path(directory) / "sample", True, 0, config)
            start.assert_not_called()
            self.assertEqual(result["status"], "invalid")
            self.assertIsNone(result["configuration_before"])
            self.assertIn("missing limit", result["error"])

    def test_drift_after_collection_invalidates_but_preserves_metrics(self):
        config = {"host": {"label": "fixture"}}

        def successful_sample(images, output, warmup, injected):
            output.mkdir()
            return {"warmup": warmup, "status": "success", "vm_boots_started": 2,
                    "metrics": {"read_seconds": .02}}

        with tempfile.TemporaryDirectory() as directory, \
                patch("measurement.series.configuration", side_effect=[config, {"changed": True}]), \
                patch("measurement.series.collect", side_effect=successful_sample):
            result = checked_sample({}, Path(directory) / "sample", False, 0, config)
            self.assertEqual(result["status"], "invalid")
            self.assertEqual(result["metrics"], {"read_seconds": .02})
            self.assertEqual(result["vm_boots_started"], 2)
            self.assertEqual(result["configuration_before"], fingerprint(config))

    def test_probe_failure_counts_only_a_started_vm_and_skips_terminal(self):
        for started in (False, True):
            with self.subTest(started=started), tempfile.TemporaryDirectory() as directory:
                def failed_probe(image, timeout, output, on_start):
                    if not started:
                        raise OSError("could not start QEMU")
                    on_start()
                    (output / "serial.log").write_text("RUSTIC PANIC")
                    return {"outcome": "panic"}

                with patch("measurement.sample.run_once", side_effect=failed_probe), \
                        patch("measurement.sample.machine") as terminal:
                    output = Path(directory) / "sample"
                    result = collect({"ok": Path("fixture.img")}, output, False, 0)
                    terminal.assert_not_called()
                    self.assertEqual(result["status"], "invalid")
                    self.assertEqual(result["vm_boots_started"], int(started))
                    self.assertEqual("probe/serial.log" in result["artifacts"], started)
                    self.assertEqual(json.loads((output / "sample.json").read_text()), result)
