# SPDX-License-Identifier: Apache-2.0
"""Reject superficially successful latency fixtures when real evidence disagrees."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.latency.evidence import RESUMED, SUSPENDED, validate


def fixture(expired=False):
    before = "id=6 parent=4 kind=file bytes=6 version=5"
    memory = "ticks=80 free_frames=52362 process_slots=8 processes=3 channels=4 pending_io=0"
    failure = ("RUSTIC BLOCK_FAILURE phase=completion request=286 kind=4 reason=Timeout started=86 now=586 "
               "elapsed_ticks=500 polls=2510 stalled_polls=0 expected=285 observed=285 device_status=7 descriptor=None status=None")
    committed = "committed id=6 previous=5 version=8 bytes=5"
    result = {
        "before": before, "after": before if expired else "id=6 parent=4 kind=file bytes=5 version=8",
        "resources_before": memory, "resources_after_completion": memory.replace("ticks=80", "ticks=600"),
        "resources_final": memory.replace("ticks=80", "ticks=610"), "clean_exit": True,
        "suspension_observed_seconds": .04, "suspended_seconds": 6.01 if expired else .61,
        "reply_seconds": 6.1 if expired else .8, "uart_before_resume": failure if expired else "replace command\r\n",
        "reply": failure + "\r\nerror: Uncertain" if expired else committed,
        "restart": "files restarted; utility sessions revoked",
        "receipt": "error: OutcomeUnknown" if expired else committed,
        "content": "before" if expired else "after", "other": "untouched",
    }
    content, version = (b"before", 5) if expired else (b"after", 8)
    state = {"files": {(4, "hello"): content, (4, "other"): b"untouched"},
             "nodes": {6: {"version": version, "content": content}}, "epoch": 1,
             "records": [] if expired else [{"id": 6, "previous": 5, "committed": 8,
                                             "content": b"after", "epoch": 1, "key": 991}]}
    serial = "rustic:/workspaces> replace hello 5 key after\r\n" + result["reply"]
    return result, state, SUSPENDED + "\n" + RESUMED + "\n", serial


class LatencyEvidence(unittest.TestCase):
    def accepted(self, expired=False, change=None):
        values = list(fixture(expired))
        if change:
            change(values)
        validate("expired-completion" if expired else "delayed-completion", *values)

    def test_both_real_completion_outcomes_are_admitted(self):
        self.accepted()
        self.accepted(True)

    def test_missing_duplicate_or_reversed_backend_events_are_rejected(self):
        for backend in ("", SUSPENDED, RESUMED, RESUMED + "\n" + SUSPENDED,
                        SUSPENDED + "\n" + SUSPENDED + "\n" + RESUMED):
            with self.subTest(backend=backend), self.assertRaises(AssertionError):
                self.accepted(change=lambda values: values.__setitem__(2, backend))

    def test_false_short_nonfinite_or_reversed_timings_are_rejected(self):
        for field, value in (("suspended_seconds", .59), ("reply_seconds", .1),
                             ("suspension_observed_seconds", -1), ("reply_seconds", float("nan")),
                             ("suspended_seconds", float("inf")), ("reply_seconds", True)):
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                self.accepted(change=lambda values: values[0].update({field: value}))

    def test_timeout_must_be_observed_before_backend_resume(self):
        with self.assertRaises(AssertionError):
            self.accepted(True, lambda values: values[0].update(uart_before_resume=""))

    def test_timeout_requires_tick_deadline_and_outstanding_flush(self):
        for old, new in (("elapsed_ticks=500", "elapsed_ticks=25"), ("kind=4", "kind=0"),
                         ("request=286", "request=0"), ("observed=285", "observed=286"),
                         ("descriptor=None", "descriptor=Some(0)"), ("status=None", "status=Some(0)"),
                         ("stalled_polls=0", "stalled_polls=5000000"), ("now=586", "now=600")):
            def change(values):
                for field in ("reply", "uart_before_resume"):
                    values[0][field] = values[0][field].replace(old, new)
                values[3] = values[3].replace(old, new)
            with self.subTest(old=old), self.assertRaises(AssertionError):
                self.accepted(True, change)

    def test_unexpected_failure_or_retry_is_rejected(self):
        for expired in (False, True):
            for extra in ("\nRUSTIC BLOCK_FAILURE", "\nrustic:/workspaces> replace hello 5 key after"):
                with self.subTest(expired=expired, extra=extra), self.assertRaises(AssertionError):
                    self.accepted(expired, lambda values: values.__setitem__(3, values[3] + extra))

    def test_resource_leak_pending_io_and_unclean_exit_are_rejected(self):
        for field, value in (("pending_io=0", "pending_io=1"), ("free_frames=52362", "free_frames=52361"),
                             ("processes=3", "processes=4"), ("channels=4", "channels=5"), ("ticks=610", "ticks=1")):
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.accepted(change=lambda values: values[0].update(
                    resources_final=values[0]["resources_final"].replace(field, value)))
        with self.assertRaises(AssertionError):
            self.accepted(change=lambda values: values[0].update(clean_exit=False))

    def test_timeout_cannot_claim_commit_or_skip_recovery(self):
        for field, value in (("reply", "committed id=6 previous=5 version=8 bytes=5"),
                             ("receipt", "committed id=6 previous=5 version=8 bytes=5"),
                             ("restart", ""), ("content", "after")):
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.accepted(True, lambda values: values[0].update({field: value}))

    def test_receipt_identity_version_bytes_and_epoch_must_match_disk(self):
        for field, value in (("id", 7), ("previous", 4), ("committed", 7),
                             ("content", b"wrong"), ("epoch", 2), ("key", 992)):
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.accepted(change=lambda values: values[1]["records"][0].update({field: value}))
        for records in ([], fixture()[1]["records"] * 2):
            with self.assertRaises(AssertionError):
                self.accepted(change=lambda values: values[1].update(records=copy.deepcopy(records)))

    def test_native_reply_and_independent_content_are_both_required(self):
        with self.assertRaises(AssertionError):
            self.accepted(change=lambda values: values[0].update(receipt="committed id=6 previous=5 version=9 bytes=5"))
        with self.assertRaises(AssertionError):
            self.accepted(change=lambda values: values[1]["files"].update({(4, "other"): b"changed"}))
        with self.assertRaises(AssertionError):
            self.accepted(True, lambda values: values[1]["nodes"][6].update(version=8))
        with self.assertRaises(AssertionError):
            self.accepted(True, lambda values: values[1].update(records=fixture()[1]["records"]))


if __name__ == "__main__":
    unittest.main()
