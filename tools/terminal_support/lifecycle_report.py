# SPDX-License-Identifier: Apache-2.0
"""Bounded native lifecycle evidence checks without schema dependencies or a VM."""
from .prevention_report import verify as verify_prevention, view as native_view

STATES = dict(prepared=1, queued=2, running=3, reconciling=4,
              succeeded=5, cancelled=6, failed=7, prevented=8)


def inspected(item, state, identity, *, stop=None, failure=None, completion=None):
    operation = item['operation']
    expected = dict(operation_id=identity['id'], service_instance=identity['instance'], state=state,
                    effect='committed' if state == 'succeeded' else 'unknown' if state == 'reconciling' else 'none')
    if stop is not None:
        assert type(operation['stop_pending']) is bool
        expected['stop_pending'] = stop
    if failure is not None:
        expected['failure'] = failure
    if completion is not None:
        expected['completion_id'] = completion
    assert operation == expected, 'contradictory logical lifecycle'
    client = item['client']
    assert all(type(v) is int for v in client.values())
    assert client == dict(status=0, value=STATES[state],
                          other=int(completion.rsplit('_', 1)[1], 16) if completion else int(bool(stop)),
                          control_denied=0, version={None: 0, 'version_conflict': 1, 'access_denied': 2}[failure])


def retained(item, identity, cause):
    state = dict(admitted='prepared', committed='succeeded').get(identity['state'])
    if state is None:
        state = dict(unknown='prevented', requested='cancelled', version_conflict='failed', authority_lost='failed')[cause]
    inspected(item, state, identity,
              failure=dict(version_conflict='version_conflict', authority_lost='access_denied').get(cause),
              completion='op_' + identity['id'][3:36] + f"{identity['terminal']:016x}" if state == 'succeeded' else None)


def acknowledgement(item, identity, disposition, *, client=False):
    expected = dict(operation_id=identity['id'], disposition=disposition)
    if client:
        assert all(type(v) is int for v in item['client'].values())
        expected['client'] = dict(status=0, value=dict(requested=1, already_requested=2, too_late=3)[disposition], other=0, control_denied=0, version=0)
    assert item == expected, 'cancel ACK exposed inspection or changed disposition'


def verify(report):
    verify_prevention(report)
    groups = report['observations']
    for name in ('legacy', 'migrated', 'prepared', 'terminal', 'reboot'):
        group = groups[name]
        assert len(group['lifecycle']) == len(group['views'])
        for item, view in zip(group['lifecycle'], group['views'], strict=True):
            retained(item, view, view['prevention'])
    group = groups['authority']['paired']
    assert len(group['lifecycle']) == 1
    retained(group['lifecycle'][0], group['views'][0], 'authority_lost')
    cases = report['lifecycle']
    assert [c['case'] for c in cases] == ['queued', 'active', 'late_lost'], 'incomplete native lifecycle inventory'
    for case in cases:
        kind = case['case']
        assert type(case['skip']) is int and case['skip'] == (15 if kind == 'late_lost' else 0)
        assert len(case['final']) == (2 if kind == 'queued' else 1)
        first, second = case['final'][0], case['final'][-1]
        assert (first['id'] != second['id']) == (kind == 'queued')
        inspected(case['prepared'], 'prepared', second)
        phase = 'reconciling' if kind == 'late_lost' else 'running'
        inspected(case['before'], phase, first, stop=False)
        inspected(case['stopped'], 'queued' if kind == 'queued' else phase, second, stop=True)
        for name in ('no_cancel_denied', 'cancel_only_denied'):
            assert all(type(v) is int for v in case[name].values())
            assert case[name] == dict(status=17, value=0, other=0, control_denied=0, version=0)
        if kind == 'late_lost':
            assert all(type(v) is int for v in case['lost_reply'].values())
            assert case['lost_reply'] == dict(status=0, value=0, other=0, control_denied=0, version=0)
        else:
            acknowledgement(case['accepted'], second, 'requested', client=True)
            acknowledgement(case['too_late_client'], second, 'too_late', client=True)
        acknowledgement(case['repeat'], second, 'already_requested')
        acknowledgement(case['too_late'], second, 'too_late')
        assert len(case['terminal']) == len(case['final'])
        disk = case['disk']
        assert len(disk['records']) == len(case['final'])
        assert type(disk['previous_version']) is int and 0 < disk['previous_version'] < first['number']
        assert type(disk['file_version']) is int
        assert disk['file_version'] == (first['terminal'] if first['state'] == 'committed' else disk['previous_version'])
        for item, final in zip(case['terminal'], case['final'], strict=True):
            expected_state = 'committed' if kind == 'late_lost' or kind == 'queued' and final == first else 'cancelled'
            assert final['state'] == expected_state and final['terminal'] > final['number']
            assert type(final['number']) is int and final['number'] == int(final['id'].rsplit('_', 1)[1], 16)
            native_view(dict(profile=2, kind='retained', id=final['id'], instance=final['instance'], state=expected_state,
                             terminal=final['terminal'], prevention='none' if expected_state == 'committed' else 'requested'),
                        expected_state, 'none' if expected_state == 'committed' else 'requested')
            retained(item, final, 'none' if expected_state == 'committed' else 'requested')
            records = [r for r in disk['records'] if r['admission'] == final['number']]
            assert records == [dict(admission=final['number'], state=expected_state, terminal=final['terminal'],
                                    prevention=None if expected_state == 'committed' else 'requested',
                                    committed=final['terminal'] if expected_state == 'committed' else 0)], 'disk and logical lifecycle disagree'
        assert len(case['disk_sha256']) == 64
    return True
