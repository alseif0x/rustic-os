# SPDX-License-Identifier: Apache-2.0
"""Host-only checker fixtures, deliberately not guest execution evidence."""
import unittest
from copy import deepcopy
from test_prevention_report import fixture as prevention_fixture
from terminal_support.lifecycle_report import verify, STATES


def projected(identity, state, stop=None, failure=None):
    value = dict(operation_id=identity['id'], service_instance=identity['instance'], state=state,
                 effect='committed' if state == 'succeeded' else 'unknown' if state == 'reconciling' else 'none')
    detail = 0
    if stop is not None:
        value['stop_pending'] = stop
        detail = int(stop)
    if state == 'succeeded':
        value['completion_id'] = 'op_' + identity['id'][3:36] + f"{identity['terminal']:016x}"
        detail = identity['terminal']
    if failure is not None:
        value['failure'] = failure
    return dict(operation=value, client=dict(status=0, value=STATES[state], other=detail,
                control_denied=0, version={None: 0, 'version_conflict': 1, 'access_denied': 2}[failure]))


def ack(identity, disposition, client=False):
    result = dict(operation_id=identity['id'], disposition=disposition)
    if client:
        result['client'] = dict(status=0, value=dict(requested=1, already_requested=2, too_late=3)[disposition], other=0, control_denied=0, version=0)
    return result


def fixture():
    report = prevention_fixture()
    for group in [report['observations'][n] for n in ('legacy','migrated','prepared','terminal','reboot')] + [report['observations']['authority']['paired']]:
        group['lifecycle'] = []
        for view in group['views']:
            cause = view['prevention']
            group['lifecycle'].append(projected(view, dict(none='prepared', unknown='prevented', requested='cancelled', version_conflict='failed', authority_lost='failed')[cause],
                failure=dict(version_conflict='version_conflict', authority_lost='access_denied').get(cause)))
    cases = []
    for n, name in enumerate(('queued','active','late_lost')):
        def record(number, state):
            return dict(id=f"ad_{'07'*16}_{number:016x}", instance=f"si_{'07'*16}_0000000000000001", state=state, number=number, terminal=number+2)
        first = record(20+4*n, 'cancelled' if name == 'active' else 'committed')
        second = record(21, 'cancelled') if name == 'queued' else first
        phase = 'reconciling' if name == 'late_lost' else 'running'
        finals = [first, second] if name == 'queued' else [first]
        case = dict(case=name, skip=15 if name=='late_lost' else 0,
                    prepared=projected(second, 'prepared'), before=projected(first, phase, False),
                    stopped=projected(second, 'queued' if name=='queued' else phase, True),
                    repeat=ack(second, 'already_requested'), too_late=ack(second, 'too_late'),
                    final=finals, terminal=[projected(v, 'succeeded' if v['state']=='committed' else 'cancelled') for v in finals], disk_sha256='a'*64)
        for denied in ('no_cancel_denied','cancel_only_denied'):
            case[denied] = dict(status=17, value=0, other=0, control_denied=0, version=0)
        if name == 'late_lost':
            case['lost_reply'] = dict(status=0, value=0, other=0, control_denied=0, version=0)
        else:
            case.update(accepted=ack(second, 'requested', True), too_late_client=ack(second, 'too_late', True))
        case['disk'] = dict(previous_version=19, file_version=first['terminal'] if first['state']=='committed' else 19,
                            records=[dict(admission=v['number'], state=v['state'], terminal=v['terminal'],
                                          prevention=None if v['state']=='committed' else 'requested',
                                          committed=v['terminal'] if v['state']=='committed' else 0) for v in finals])
        cases.append(case)
    report['lifecycle'] = cases
    return report


class LifecycleReport(unittest.TestCase):
    def test_complete_bounded_report(self):
        self.assertTrue(verify(fixture()))

    def test_missing_cases_and_speculative_terminal_results_are_rejected(self):
        mutations = [lambda r:r['lifecycle'].pop(),
                     lambda r:r['lifecycle'][2]['terminal'][0]['operation'].update(stop_pending=True),
                     lambda r:r['lifecycle'][2]['terminal'][0]['operation'].update(operation_id=r['lifecycle'][2]['terminal'][0]['operation']['completion_id']),
                     lambda r:r['lifecycle'][0]['stopped']['operation'].update(state='cancelled'),
                     lambda r:r['lifecycle'][0]['accepted'].update(operation={}),
                     lambda r:r['lifecycle'][0]['cancel_only_denied'].update(other=1),
                     lambda r:r['lifecycle'][2]['before']['operation'].update(effect='none'),
                     lambda r:r['lifecycle'][0]['disk']['records'][1].update(prevention='unknown'),
                     lambda r:r['lifecycle'][1]['disk'].update(file_version=99),
                     lambda r:r['observations']['legacy']['lifecycle'][0]['operation'].update(state='cancelled')]
        for mutate in mutations:
            report = deepcopy(fixture())
            mutate(report)
            with self.assertRaises(AssertionError):
                verify(report)
