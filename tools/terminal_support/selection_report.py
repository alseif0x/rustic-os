# SPDX-License-Identifier: Apache-2.0
"""Offline contradictions in the bounded one-client native mission evidence."""
import hashlib
import re
from .lifecycle_report import inspected, acknowledgement


def empty(value, status):
    assert all(type(v) is int for v in value.values())
    assert value == dict(status=status, value=0, other=0, control_denied=0, version=0), 'guard disclosed data or allowed replay'


def verify(cases):
    assert [c['case'] for c in cases] == ['complete', 'cancel', 'revoke', 'conflict'], 'selected mission inventory'
    clients = [c['client'] for c in cases]
    assert all(type(c) is int and c > 0 for c in clients) and len(set(clients)) == 4
    for c in cases:
        kind = c['case']
        assert type(c['rights']) is int and c['rights'] == 15
        assert len(c['selection']) == 2
        for value, method in zip(c['selection'], (5, 6), strict=True):
            assert all(type(v) is int for v in value.values())
            assert value == dict(status=0, value=1, other=method, control_denied=1, version=2), 'wrong selected method/profile'
        resources = c['resources']
        assert all(type(v) is int for v in resources) and resources == [3, 4, 4, 6], 'mission used extra clients'
        prepared = c['prepared_view']['operation']
        assert re.fullmatch('ad_[0-9a-f]{32}_[0-9a-f]{16}', prepared['operation_id'])
        assert re.fullmatch('si_[0-9a-f]{32}_[0-9a-f]{16}', prepared['service_instance'])
        identity = dict(id=prepared['operation_id'], instance=prepared['service_instance'])
        assert identity['id'][3:35] == identity['instance'][3:35]
        number = int(identity['id'].rsplit('_', 1)[1], 16)
        inspected(c['prepared_view'], 'prepared', identity)
        if kind == 'conflict':
            assert 'running' not in c, 'conflicted candidate falsely reported executing'
        else:
            inspected(c['running'], 'running', identity, stop=False)
        assert all(type(v) is int for v in c['scheduled'].values())
        assert c['scheduled'] == dict(status=0, value=4, other=0, control_denied=0, version=0)
        guards = c['guards']
        assert len(guards['unprepared']) == 4
        for value in guards['unprepared']:
            empty(value, 4)
        for name in ('prepare', 'schedule', 'cancel', 'terminal_prepare', 'terminal_schedule'):
            empty(guards[name], 20)
        disk = c['disk']
        record = disk['record']
        assert all(type(disk[k]) is int for k in ('previous', 'version', 'epoch', 'length'))
        assert 0 < disk['previous'] < number < record['terminal'] and disk['epoch'] > 0
        assert disk['length'] == len(b'Hello from native Rust')
        assert all(type(v) is int for v in c['prepared'].values())
        assert c['prepared'] == dict(status=0, value=number, other=disk['epoch'], control_denied=disk['length'], version=disk['previous']), 'prepared candidate did not use observed version/epoch'
        committed = kind == 'complete'
        cause = None if committed else dict(cancel='requested', revoke='authority_lost', conflict='version_conflict')[kind]
        assert all(type(record[k]) is int for k in ('admission', 'terminal', 'committed'))
        assert record == dict(admission=number, state='committed' if committed else 'cancelled',
                              terminal=record['terminal'], prevention=cause, committed=record['terminal'] if committed else 0), 'disk contradicts effect/cause'
        expected_version = record['terminal'] if committed else c['human']['version'] if kind == 'conflict' else disk['previous']
        assert disk['version'] == expected_version, 'false rollback or committed version'
        content = b'Single native client' if committed else b'Human edit wins' if kind == 'conflict' else b'Hello from native Rust'
        assert disk['sha256'] == hashlib.sha256(content).hexdigest(), 'wrong independently read file content'
        if committed:
            completion = f"op_{identity['id'][3:35]}_{record['terminal']:016x}"
            inspected(c['terminal'], 'succeeded', identity, completion=completion)
            assert all(type(v) is int for v in c['readback'].values())
            assert c['readback'] == dict(status=0, value=len(content), other=disk['epoch'], control_denied=1, version=expected_version), 'client did not verify committed readback'
            newer = c['superseded']
            assert type(newer['version']) is int and newer['version'] > record['terminal']
            assert newer['sha256'] == disk['sha256'], 'supersession must use identical bytes'
            empty(newer['rejected'], 1)
        elif kind == 'cancel':
            busy = c['refresh_busy']
            assert all(type(v) is int for v in busy.values())
            assert busy == dict(status=20, value=0, other=0, control_denied=0, version=0), 'missing busy-refresh boundary'
            acknowledgement(c['ack'], identity, 'requested', client=True)
            inspected(c['terminal'], 'cancelled', identity)
        elif kind == 'revoke':
            assert len(c['denied']) == 2
            for value in c['denied']:
                assert all(type(v) is int for v in value.values())
                assert value == dict(status=18, value=0, other=0, control_denied=0, version=0), 'stale authority'
            assert c['terminal'] == dict(operation=dict(operation_id=identity['id'], service_instance=identity['instance'],
                state='failed', effect='none', failure='access_denied')), 'owner reconciliation lost authority cause'
        else:
            assert type(c['human']['version']) is int and number < c['human']['version'] < record['terminal']
            assert c['human']['sha256'] == disk['sha256'], 'candidate overwrote human bytes'
            inspected(c['terminal'], 'failed', identity, failure='version_conflict')
        if not committed:
            empty(c['not_succeeded'], 18 if kind == 'revoke' else 4)
    return True
