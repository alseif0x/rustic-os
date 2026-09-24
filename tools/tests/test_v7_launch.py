# SPDX-License-Identifier: Apache-2.0
"""Storage launch output from the V7 guest must decode to exactly one outcome."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_launch import (
    ROLE_REFUSAL,
    counters,
    manifest_facts,
    permissions_facts,
    process_rows,
    reap_outcome,
    start_outcome,
)


PROMPT = "\r\nrustic:/workspaces> "


class StartOutcome(unittest.TestCase):
    def test_success_names_the_child_role_and_topology(self):
        text = "start-staged 4 exit\r\nstarted staged pid=4 role=exit topology=control-only" + PROMPT
        self.assertEqual(start_outcome(text), {"state": "started", "pid": 4, "role": "exit"})

    def test_refusals_and_denial_are_not_success(self):
        refused = f"start-staged 4 read\r\nerror: start refused: {ROLE_REFUSAL}" + PROMPT
        self.assertEqual(start_outcome(refused), {"state": "refused", "reason": ROLE_REFUSAL})
        kernel = "start-staged 5 exit\r\nerror: start refused: kernel Full" + PROMPT
        self.assertEqual(start_outcome(kernel), {"state": "refused", "reason": "kernel Full"})
        denied = "start-staged 1 exit\r\nerror: service denied" + PROMPT
        self.assertEqual(start_outcome(denied), {"state": "denied"})

    def test_other_and_ambiguous_output_is_rejected(self):
        for text in (
            "start-staged 4 exit" + PROMPT,
            "started staged pid=4 role=exit topology=control-only\r\nerror: service unavailable\r\n",
            "started staged pid=4 role=exit\r\n",
            "started staged pid=4 role=exit topology=full\r\n",
            "error: stage refused: pair mismatch\r\n",
            "error: service busy or full\r\n",
            "error: invalid arguments; type help\r\n",
        ):
            with self.assertRaises(ValueError, msg=text):
                start_outcome(text)


class Observations(unittest.TestCase):
    def test_permissions_words_decode_in_order(self):
        text = "permissions 4\r\nscope=0 rights=0 generation=1 expires=0 report=7 bytes=2 other=0" + PROMPT
        self.assertEqual(permissions_facts(text), {
            "scope": 0, "rights": 0, "generation": 1, "expires": 0, "report": 7, "bytes": 2, "other": 0,
        })
        for text in ("permissions 9\r\nerror: service denied" + PROMPT, "scope=0 rights=0\r\n"):
            with self.assertRaises(ValueError, msg=text):
                permissions_facts(text)

    def test_reap_requires_one_successful_answer(self):
        self.assertEqual(reap_outcome("reap 4\r\nok exit_kind=1 code=7" + PROMPT), (1, 7))
        for text in ("reap 4\r\nerror: service denied" + PROMPT, "reap 4\r\nok" + PROMPT):
            with self.assertRaises(ValueError, msg=text):
                reap_outcome(text)

    def test_process_rows_keep_exit_kind_and_code(self):
        text = ("ps\r\nPID STATE EXIT CODE PREEMPTIONS PARENT PROGRAM\r\n1 running 0 0 3 0 supervisor\r\n"
                "4 exited 1 7 0 1 staged\r\n5 dormant 0 0 0 1 staged" + PROMPT)
        rows = process_rows(text)
        self.assertEqual(rows[4], {"state": "exited", "kind": 1, "code": 7, "program": "staged"})
        self.assertEqual(rows[5]["state"], "dormant")
        self.assertEqual(rows[1]["program"], "supervisor")

    def test_counters_need_processes_and_channels(self):
        text = "ticks=9 free_frames=1 process_slots=8 processes=3 channels=4 pending_io=0 heap_pages=2"
        self.assertEqual(counters(text), {"processes": 3, "channels": 4})
        with self.assertRaises(ValueError):
            counters("ticks=9 processes=3")


class Manifest(unittest.TestCase):
    def test_schema_two_manifest_facts(self):
        import application
        document = {"schema": 2, "identity": "rustic.utility", "executable": "utility.elf",
                    "version": [0, 1, 0], "process_abi": 65536, "ipc_version": 1, "requests": ["ipc"]}
        digest = bytes(range(32))
        self.assertEqual(manifest_facts(application.encode(document, digest)), {
            "identity": "rustic.utility", "version": [0, 1, 0], "requests": 1,
            "artifact_sha256": digest.hex(),
        })
        with self.assertRaises(ValueError):
            manifest_facts(b"RUSTAPP\0" + bytes(119))


if __name__ == "__main__":
    unittest.main()
