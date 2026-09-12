# SPDX-License-Identifier: Apache-2.0
"""Offline structural checks for negotiated native lifecycle evidence; no VM imports."""
import re
from .lifecycle_report import inspected, acknowledgement

METHODS = ('operations.get', 'operations.cancel')


def digest(value):
    assert isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value), 'missing digest'


def profiles(items, available, client=False):
    assert len(items) == 2 and [v['method'] for v in items] == list(METHODS), 'profile inventory'
    for item in items:
        assert set(item) == {'method','version','profile','availability','responder','context','retained','tickets','active','sha256'} | ({'client'} if client else set())
        for key in ('version','profile','responder','context','retained','tickets','active'):
            assert type(item[key]) is int, 'noninteger descriptor'
        assert (item['version'], item['profile']) == (2, 1), 'wrong negotiated version/profile'
        assert item['availability'] == ('available' if available else 'unavailable'), 'false support'
        assert item['responder'] > 0 and item['context'] > 0, 'missing live binding'
        assert (item['retained'], item['tickets'], item['active']) == (2, 2, 1), 'wrong enforced limits'
        digest(item['sha256'])
        if client:
            value = item['client']
            assert all(type(v) is int for v in value.values())
            assert value == dict(status=0, value=(1 if available else 3) | (2<<8) | (2<<16) | (1<<24),
                                 other=item['responder'], control_denied=1, version=2), 'client profile mismatch'
    assert items[0]['responder'] == items[1]['responder'] and items[0]['context'] == items[1]['context']


def denial(value, status):
    assert all(type(v) is int for v in value.values())
    assert value == dict(status=status, value=0, other=0, control_denied=0, version=0), 'denial disclosed data'


def verify(report):
    initial, legacy, restart, reboot = (report[n] for n in ('before_upgrade','legacy','restart','reboot'))
    profiles(initial['profiles'], False)
    profiles(legacy['profiles'], True, True)
    profiles(restart['before'], True, True)
    profiles(restart['after'], True)
    profiles(reboot['profiles'], True)
    for group in (initial, restart, reboot):
        digest(group['disk_sha256'])
    expected_hashes = [v['sha256'] for v in initial['profiles']]
    for items in (legacy['profiles'], restart['before'], restart['after'], reboot['profiles']):
        assert [v['sha256'] for v in items] == expected_hashes, 'contract changed across support/restart'
    prepared = legacy['prepared']['operation']
    identity = dict(id=prepared['operation_id'], instance=prepared['service_instance'])
    inspected(legacy['prepared'], 'prepared', identity)
    inspected(legacy['terminal'], 'prevented', identity)
    acknowledgement(legacy['accepted'], identity, 'requested', client=True)
    acknowledgement(legacy['too_late'], identity, 'too_late')
    denial(legacy['inspect_denied'], 17)
    denial(legacy['cancel_denied'], 17)
    record = legacy['disk']
    number = int(identity['id'].rsplit('_',1)[1],16)
    assert type(record['terminal']) is int and record['terminal'] > number
    assert type(legacy['format']) is int and legacy['format'] == 4, 'wrong legacy format'
    assert record == dict(admission=number,state='cancelled',terminal=record['terminal'],committed=0), 'v4 does not persist a cause'
    denial(restart['revoked'], 18)
    assert restart['selection_reset'] == ['inspect-selected', 'cancel-selected'], 'rebind retained selected support'
    assert type(restart['retired_client']) is int and restart['retired_client'] > 0
    assert restart['before'][0]['responder'] != restart['after'][0]['responder'], 'stale responder'
    operation = restart['operation']['operation']
    inspected(restart['operation'], 'cancelled', dict(id=operation['operation_id'], instance=operation['service_instance']))
    assert restart['after_operation'] == reboot['operation'] == dict(operation=operation), 'historical origin changed'
    return True
