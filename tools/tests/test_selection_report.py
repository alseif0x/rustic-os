# SPDX-License-Identifier: Apache-2.0
"""Synthetic evidence checker tests, never evidence that a guest ran."""
import hashlib
import unittest
from copy import deepcopy
from test_lifecycle_report import projected, ack
from terminal_support.selection_report import verify


def fixture():
    cases = []
    for index, kind in enumerate(('complete', 'cancel', 'revoke')):
        identity = dict(id='ad_'+'07'*16+f'_{9+index:016x}', instance='si_'+'07'*16+'_0000000000000001', terminal=20)
        completed = kind == 'complete'
        content = b'Single native client' if completed else b'Hello from native Rust'
        cause = None if completed else 'requested' if kind == 'cancel' else 'authority_lost'
        terminal = projected(identity, 'succeeded' if completed else 'cancelled' if kind == 'cancel' else 'failed',
                             failure='access_denied' if kind == 'revoke' else None)
        if kind == 'revoke':
            terminal.pop('client')
        case = dict(case=kind, client=index+10, rights=15,
                    selection=[dict(status=0, value=1, other=method, control_denied=1, version=2) for method in (5,6)],
                    prepared=dict(status=0, value=9+index, other=1, control_denied=22, version=2),
                    running=projected(identity, 'running', stop=False), terminal=terminal,
                    resources=[3,4,4,6], disk=dict(previous=2, version=20 if completed else 2, epoch=1, length=22,
                        sha256=hashlib.sha256(content).hexdigest(), record=dict(admission=9+index, state='committed' if completed else 'cancelled', terminal=20, prevention=cause, committed=20 if completed else 0)))
        if completed:
            case['readback'] = dict(status=0, value=len(content), other=1, control_denied=1, version=20)
        elif kind == 'cancel':
            case['ack'] = ack(identity, 'requested', True)
        else:
            case['denied'] = [dict(status=18, value=0, other=0, control_denied=0, version=0) for _ in range(2)]
        cases.append(case)
    return cases


class SelectionReport(unittest.TestCase):
    def test_single_client_completion_stop_and_revocation(self):
        self.assertTrue(verify(fixture()))

    def test_false_effect_stale_authority_or_missing_selection_is_rejected(self):
        mutations = [
            lambda c:c.pop(),
            lambda c:c[0].update(resources=[3,5,4,8]),
            lambda c:c[0]['selection'].pop(),
            lambda c:c[0]['selection'][1].update(other=5),
            lambda c:c[0]['selection'][0].update(version=1),
            lambda c:c[0]['prepared'].update(version=1),
            lambda c:c[0]['prepared'].update(other=2),
            lambda c:c[0]['running']['client'].update(value=2),
            lambda c:c[0]['terminal']['operation'].update(effect='none'),
            lambda c:c[0]['readback'].update(control_denied=0),
            lambda c:c[0]['disk'].update(sha256='a'*64),
            lambda c:c[1]['disk'].update(version=20),
            lambda c:c[1]['disk']['record'].update(committed=20),
            lambda c:c[1]['ack'].update(disposition='too_late'),
            lambda c:c[2]['disk']['record'].update(prevention='requested'),
            lambda c:c[2]['denied'][1].update(status=0),
            lambda c:c[2]['terminal']['operation'].update(failure='version_conflict'),
            lambda c:c[0].update(client=True),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                cases = deepcopy(fixture())
                mutate(cases)
                with self.assertRaises((AssertionError, KeyError)):
                    verify(cases)
