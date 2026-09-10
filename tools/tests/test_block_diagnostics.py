# SPDX-License-Identifier: Apache-2.0
"""Failure admission requires evidence of the exercised driver path and recovery."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from boot_support.block_evidence import verified
from boot_support.scenarios import records


def summary(phase):
    return (f"RUSTIC BLOCK verified=1 phase={phase} sectors=8388608 sector_bytes=512 "
            "max_bytes=512 dma_frames=3 rejected=5 free_before=90 free_after=90")


def failure(**changes):
    value = {"phase": "completion", "request": 0, "kind": 0, "reason": "Timeout",
             "started": 10, "now": 510, "elapsed_ticks": 500, "polls": 100,
             "stalled_polls": 0,
             "expected": 0, "observed": 0, "device_status": 7,
             "descriptor": "None", "status": "None"}
    value.update(changes)
    return "RUSTIC BLOCK_FAILURE " + " ".join(f"{key}={item}" for key, item in value.items())


def errors():
    return [failure(kind=1, reason="Io", started=10, now=12, elapsed_ticks=2,
                    observed=1, descriptor="Some(0)", status="Some(1)"),
            failure(kind=65535, reason="Unsupported", started=12, now=13, elapsed_ticks=1,
                    expected=1, observed=2, descriptor="Some(0)", status="Some(2)")]


class BlockDiagnostics(unittest.TestCase):
    def admitted(self, mode, diagnostics, phase):
        return verified(mode, diagnostics + "\n" + summary(phase), records)

    def test_failure_summary_cannot_replace_missing_or_duplicate_diagnostics(self):
        for mode, phase, lines in [("block-timeout", "timeout", [failure()]),
                                   ("block-error", "error", errors())]:
            with self.subTest(mode=mode):
                self.assertTrue(self.admitted(mode, "\n".join(lines), phase))
                for bad in [[], lines[:-1], lines + [lines[0]]]:
                    self.assertFalse(self.admitted(mode, "\n".join(bad), phase))
                self.assertFalse(verified(mode, "\n".join(lines), records))
                self.assertFalse(verified(mode, "\n".join(lines) + "\n"
                                          + summary(phase).replace("free_after=90", "free_after=89"), records))

    def test_timeout_requires_no_used_entry_and_an_actual_expired_bound(self):
        self.assertTrue(self.admitted("block-timeout", failure(), "timeout"))
        self.assertTrue(self.admitted("block-timeout", failure(now=10, elapsed_ticks=0,
                                                             polls=5_000_000, stalled_polls=5_000_000), "timeout"))
        self.assertTrue(self.admitted("block-timeout", failure(polls=10_000_000), "timeout"))
        for changes in [{"now": 509, "elapsed_ticks": 499}, {"polls": 0},
                        {"now": 35, "elapsed_ticks": 25, "polls": 5_000_000},
                        {"stalled_polls": 101}, {"stalled_polls": 5_000_001, "polls": 5_000_001},
                        {"observed": 1}, {"expected": 1},
                        {"descriptor": "Some(0)"}, {"status": "Some(0)"},
                        {"reason": "Io"}, {"request": 1}, {"kind": 4}, {"device_status": 0}]:
            with self.subTest(changes=changes):
                self.assertFalse(self.admitted("block-timeout", failure(**changes), "timeout"))

    def test_device_errors_require_distinct_statuses_and_ring_progress(self):
        first, second = errors()
        for bad in [[second, first], [first, first],
                    [first.replace("status=Some(1)", "status=Some(0)"), second],
                    [first, second.replace("observed=2", "observed=1")],
                    [first, second.replace("expected=1", "expected=0")],
                    [first, second.replace("descriptor=Some(0)", "descriptor=Some(1)")],
                    [first, second.replace("kind=65535", "kind=0")],
                    [first, second.replace("started=12", "started=11").replace("elapsed_ticks=1", "elapsed_ticks=2")]]:
            with self.subTest(bad=bad):
                self.assertFalse(self.admitted("block-error", "\n".join(bad), "error"))

    def test_observed_device_error_takes_precedence_over_elapsed_timeout(self):
        first, second = errors()
        late = second.replace("now=13", "now=612").replace("elapsed_ticks=1", "elapsed_ticks=600")
        self.assertTrue(self.admitted("block-error", first + "\n" + late, "error"))

    def test_malformed_duplicate_or_incoherent_fields_cannot_be_hidden(self):
        good = failure()
        for bad in [good.replace(" now=510", ""), good + " polls=100", good + " extra=1",
                    good + " incomplete", good.replace("polls=100", "polls=no"),
                    good.replace("now=510", "now=9"), good.replace("elapsed_ticks=500", "elapsed_ticks=499"),
                    good.replace("observed=0", "observed=65536"), good.replace("request=0", "request=-1"),
                    good.replace("polls=100", "polls=+100"), good.replace("polls=100", "polls=0100"),
                    good.replace("started=10", "started=18446744073709551616"),
                    good + "\nRUSTIC BLOCK_FAILURE", good.replace("phase=completion", "phase=start")]:
            with self.subTest(bad=bad):
                self.assertFalse(self.admitted("block-timeout", bad, "timeout"))

    def test_unexpected_failure_cannot_hide_in_successful_readonly_fixture(self):
        self.assertTrue(self.admitted("block-readonly", "", "readonly"))
        self.assertFalse(self.admitted("block-readonly", failure(), "readonly"))
