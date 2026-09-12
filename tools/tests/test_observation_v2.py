# SPDX-License-Identifier: Apache-2.0
import unittest
from unittest.mock import Mock, patch
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support import observations


class DetailedObservation(unittest.TestCase):
    def setUp(self):
        self.identity = dict(id='ad_' + '07'*16 + '_0000000000000009',
                             instance='si_' + '07'*16 + '_0000000000000008',
                             lineage='07'*16, terminal=12)
        self.line = (f"admission-observation-v2 profile=2 id={self.identity['id']} "
                     f"service_instance={self.identity['instance']} kind=retained "
                     "state=cancelled terminal=12 prevention=requested")

    def test_profile_cause_and_terminal_must_match_exact_native_response(self):
        for line in (self.line, self.line.replace('v2 profile=2', 'v1 profile=1'),
                     self.line.replace('requested', 'unknown'), self.line.replace('terminal=12', 'terminal=13'),
                     self.line.replace(' prevention=requested', ''), self.line + '\n' + self.line,
                     self.line + '\ncompletion=op_fake', self.line + '\nerror: Denied'):
            uart = Mock()
            uart.command.return_value = line
            if line == self.line:
                view = observations.observe(uart, self.identity, 'retained', 'cancelled', profile=2, prevention='requested')
                self.assertEqual(view['prevention'], 'requested')
            else:
                with self.assertRaises(AssertionError):
                    observations.observe(uart, self.identity, 'retained', 'cancelled', profile=2, prevention='requested')

    def test_deterministic_cause_cannot_be_omitted_or_changed(self):
        view = dict(profile=2, kind='retained', state='cancelled', id=self.identity['id'],terminal=12,prevention='requested')
        for cause in range(5):
            result = dict(value=2,other=12,control_denied=0,version=cause)
            with patch.object(observations, 'act', return_value=result) as act:
                if cause == 2:
                    self.assertEqual(observations.paired(None, 42, view), result)
                else:
                    with self.assertRaises(AssertionError):
                        observations.paired(None, 42, view)
                act.assert_called_once_with(None, 42, 'observe-v2', view['id'])

    def test_live_or_committed_cause_cannot_be_asserted_as_a_confirmed_prevention(self):
        for kind, state, reason in [('active','running','requested'),('retained','committed','requested'),('retained','cancelled','none')]:
            with self.assertRaises(AssertionError):
                observations.observe(Mock(), self.identity, kind, state, profile=2, prevention=reason)
