# SPDX-License-Identifier: Apache-2.0
"""The native fault oracle must observe uncertainty, not hide arbitrary guest failure."""
import sys
from pathlib import Path
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.scheduled_failures.evidence import await_uncertain


class ScheduledFailureResponse(unittest.TestCase):
    def test_waits_only_for_busy_and_requires_one_unambiguous_uncertain_result(self):
        uart = Mock(command=Mock(side_effect=['error: Busy\r\n', 'error: Uncertain\r\n']))
        with patch('terminal_support.scheduled_failures.evidence.time.sleep'):
            self.assertEqual(await_uncertain(uart, {'id': 'ad_fixture'}), 'Uncertain')
        self.assertEqual(uart.command.call_count, 2)
        for response in ('error: Closed\n', 'error: Protocol\n', 'admission-v1 state=cancelled\n',
                         'error: Uncertain\nerror: Closed\n', 'error: Busy\nadmission-v1 state=committed\n',
                         'error: Uncertain\nadmission-v1 state=committed\n'):
            with self.assertRaises(AssertionError):
                await_uncertain(Mock(command=Mock(return_value=response)), {'id': 'ad_fixture'})

    def test_busy_does_not_create_an_unbounded_wait(self):
        uart = Mock(command=Mock(return_value='error: Busy\n'))
        with patch('terminal_support.scheduled_failures.evidence.time.monotonic', side_effect=[0, 13]):
            with self.assertRaises(AssertionError):
                await_uncertain(uart, {'id': 'ad_fixture'})
        self.assertEqual(uart.command.call_count, 1)
