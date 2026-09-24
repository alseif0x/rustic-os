# SPDX-License-Identifier: Apache-2.0
"""The V7 capacity harness must plan the fill exactly and decode timed restarts strictly."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.v7_capacity import (BIG_BYTES, FREE_TARGET, READ_BYTES, RECLAIM_SIZE, REFUSED_SIZES, SECTOR,
                                          SMALL_FILES, WRITE_SIZE, decode_restart, plan_big, sectors, small_content)


def restart(ticks=2512):
    return ("restart files timed\r\nfiles restarted; utility sessions revoked\r\n"
            f"restart-files ticks={ticks}\r\nrustic:/workspaces> ")


class Plan(unittest.TestCase):
    def test_the_fill_leaves_exactly_the_target_free(self):
        for free in (FREE_TARGET, FREE_TARGET + 1, FREE_TARGET + 1024, 130_000):
            full, tail = plan_big(free)
            self.assertEqual(free - full * (BIG_BYTES // SECTOR) - sectors(tail), FREE_TARGET)
            self.assertLess(tail, BIG_BYTES)
            self.assertEqual(tail % SECTOR, 0)
        self.assertEqual(plan_big(FREE_TARGET), (0, 0))
        with self.assertRaises(ValueError):
            plan_big(FREE_TARGET - 1)

    def test_write_sizes_straddle_the_free_space_they_are_meant_to(self):
        # Refused on the full volume; fits it; the reclaim write is refused
        # after two small writes and fits once the first one's snapshot is freed.
        self.assertTrue(all(sectors(size) > FREE_TARGET for size in REFUSED_SIZES))
        after_two = FREE_TARGET - 2 * sectors(WRITE_SIZE)
        self.assertGreater(sectors(RECLAIM_SIZE), after_two)
        self.assertLessEqual(sectors(RECLAIM_SIZE), after_two + sectors(WRITE_SIZE))

    def test_small_files_fit_one_read_and_differ(self):
        contents = [small_content(index) for index in range(SMALL_FILES)]
        self.assertTrue(all(0 < len(item) <= READ_BYTES for item in contents))
        self.assertEqual(len(set(contents)), SMALL_FILES)


class Restart(unittest.TestCase):
    def test_the_restart_ticks_are_decoded(self):
        self.assertEqual(decode_restart(restart()), {"ticks": 2512})
        self.assertEqual(decode_restart(restart(0)), {"ticks": 0})

    def test_a_single_error_is_a_refusal(self):
        self.assertEqual(decode_restart("restart files timed\r\nerror: service unavailable\r\n> ")["error"],
                         "service unavailable")

    def test_incomplete_or_ambiguous_answers_are_rejected(self):
        for text in (
            restart().replace("restart-files ticks=2512\r\n", ""),
            restart().replace("files restarted; utility sessions revoked\r\n", ""),
            restart() + restart(),
            restart().replace("ticks=2512", "ticks=02512"),
            restart().replace("ticks=2512", "ticks="),
            restart().replace("rustic:", "error: Busy\r\nrustic:"),
        ):
            with self.assertRaises(ValueError):
                decode_restart(text)


if __name__ == "__main__":
    unittest.main()
