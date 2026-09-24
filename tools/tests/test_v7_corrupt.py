# SPDX-License-Identifier: Apache-2.0
"""Guest output the V7 corrupt-input harness relies on must decode unambiguously."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_corrupt import (
    EXIT_CODE,
    FILE_STARTUP_CORRUPT,
    MOUNT_JOB,
    MOUNT_JOB_KIND,
    MOUNT_REFUSED,
    processes,
)


# Recorded from a `terminal-v7` boot on a volume with one flipped ELF payload byte.
REFUSED_PS = ("ps\r\nPID STATE EXIT CODE PREEMPTIONS PARENT PROGRAM\r\n1 running 0 0 3 0 supervisor\r\n"
              "2 ready 0 0 1 1 shell\r\n3 exited 1 5 3 1 files\r\nrustic:/workspaces> ")
REFUSED_JOB = ("job-status 1\r\njob=1 complete kind=11 status=4 value=0 token=0 generation=0\r\n"
               "error: service unavailable\r\nrustic:/workspaces> ")
MOUNTED_JOB = "job-status 1\r\njob=1 complete kind=11 status=0 value=3 token=8 generation=2\r\nrustic:/workspaces> "


class CorruptOutputTests(unittest.TestCase):
    def test_a_refused_mount_leaves_an_exited_file_service_with_the_corrupt_status(self):
        rows = processes(REFUSED_PS)
        self.assertEqual(rows[3], {"state": "exited", "exit": EXIT_CODE, "code": FILE_STARTUP_CORRUPT,
                                   "program": "files"})
        self.assertEqual({row["program"] for row in rows.values()}, {"supervisor", "shell", "files"})

    def test_mount_job_status_distinguishes_refusal_from_success(self):
        refused = MOUNT_JOB.search(REFUSED_JOB)
        self.assertEqual((int(refused[1]), int(refused[2]), int(refused[3])), (1, MOUNT_JOB_KIND, MOUNT_REFUSED))
        mounted = MOUNT_JOB.search(MOUNTED_JOB)
        self.assertEqual(int(mounted[3]), 0)
        self.assertIsNone(MOUNT_JOB.search("job=1 pending kind=11 phase=3 service=0 pending_io=0\r\n"))


if __name__ == "__main__":
    unittest.main()
