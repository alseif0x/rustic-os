# SPDX-License-Identifier: Apache-2.0
"""Stage job output from the V7 guest must decode to exactly one outcome."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_read import STAGE_KIND, stage_outcome, version_text


ELF = "v_0000000000000003"
MANIFEST = "v_0000000000000004"


def staged(job=4, kind=STAGE_KIND, pid=5, elf=ELF, manifest=MANIFEST):
    return (f"job-status {job}\r\njob={job} complete kind={kind} status=0 staged pid={pid} "
            f"elf_version={elf} manifest_version={manifest} state=dormant\r\nrustic:/workspaces> ")


class StageOutcome(unittest.TestCase):
    def test_success_reports_the_dormant_child_and_both_pins(self):
        self.assertEqual(stage_outcome(staged()), {
            "job": 4, "state": "staged", "pid": 5, "elf_version": ELF, "manifest_version": MANIFEST,
        })

    def test_pending_and_refusal_are_not_success(self):
        pending = f"job=7 pending kind={STAGE_KIND} phase=1 service=3 pending_io=0\r\n> "
        self.assertEqual(stage_outcome(pending), {"job": 7, "state": "pending", "phase": 1})
        copying = f"job=7 pending kind={STAGE_KIND} phase=2 service=3 pending_io=0\r\n> "
        self.assertEqual(stage_outcome(copying)["phase"], 2)
        refused = (f"job=8 complete kind={STAGE_KIND} status=45\r\n"
                   "error: stage refused: file Version\r\n> ")
        self.assertEqual(stage_outcome(refused), {
            "job": 8, "state": "refused", "status": 45, "reason": "file Version",
        })
        superseded = (f"job=9 complete kind={STAGE_KIND} status=6\r\n"
                      "error: stage refused: superseded by a service restart; transaction aborted\r\n")
        self.assertEqual(stage_outcome(superseded)["status"], 6)

    def test_other_jobs_and_ambiguous_output_are_rejected(self):
        for text in (
            staged(kind=STAGE_KIND - 1),
            f"job=7 pending kind=11 phase=1 service=3 pending_io=0\r\n",
            staged() + "\r\nerror: service unavailable\r\n",
            staged(elf="v_3"),
            f"job=8 complete kind={STAGE_KIND} status=45\r\n",
            "job=8 complete kind=11 status=45\r\nerror: stage refused: file Version\r\n",
            "error: service busy or full\r\n",
        ):
            with self.assertRaises(ValueError, msg=text):
                stage_outcome(text)

    def test_versions_use_the_canonical_reference_text(self):
        self.assertEqual(version_text(3), ELF)
        self.assertEqual(version_text(1 << 60), "v_1000000000000000")


if __name__ == "__main__":
    unittest.main()
