# SPDX-License-Identifier: Apache-2.0
"""Reject false rollback, replay and missing uncertainty in scheduled failure evidence."""
import hashlib
import re
from .validation import require
from .activity_conformance import identity, live_operation, check_operation, uncertain_operation

PREFIX = 'scheduled_failure_'
BASE = ((0, 'queued', 0, 0), (0, 'running', 1, 0), (1, 'queued', 0, 0))
LATE = (BASE[0], (0, 'settling', 1, 0), BASE[2])
CASES = {
    'late_header': (('committed', 'cancelled'), LATE + ((0, 'settling', 1, 1),), b'first', 15),
    'late_flush': (('committed', 'cancelled'), LATE + ((0, 'settling', 1, 1),), b'first', 16),
    'lost_stop': (('cancelled', 'committed'), BASE + ((0, 'stopping', 1, 1),), b'second', None),
    'lost_result': (('committed', 'cancelled'), BASE, b'first', None),
    'restart_data': (('admitted', 'admitted'), BASE, b'before', 0),
    'restart_flush': (('committed', 'admitted'), LATE, b'first', 16),
    'io_data': (('admitted', 'admitted'), BASE + ((0, 'stopping', 1, 1),), b'before', 0),
    'io_flush': (('committed', 'admitted'), LATE + ((0, 'settling', 1, 1),), b'first', 16),
    'pressure': (('cancelled', 'cancelled'), BASE + ((1, 'queued', 0, 1), (0, 'stopping', 1, 1)), b'before', None),
}


def version(value):
    return isinstance(value, str) and re.fullmatch(r'v_[0-9a-f]{16}', value) and int(value[2:], 16) > 0


def check_scheduled_failures(catalog, cases):
    found = [c for c in cases if isinstance(c, dict) and str(c.get('case', '')).startswith(PREFIX)]
    require(len(found) == len(CASES) and {c['case'] for c in found} == {PREFIX + n for n in CASES},
            'missing, duplicate or unknown scheduled failure cut')
    for case in found:
        name = case['case'][len(PREFIX):]
        states, expected, content, skip = CASES[name]
        require(all(case.get(f) is True for f in ('verified', 'reboot_verified', 'no_replay')),
                'unverified scheduled failure or replay check')
        require(isinstance(case.get('sha256'), str) and re.fullmatch(r'[0-9a-f]{64}', case['sha256']),
                'missing independent recovery volume identity')
        if skip is not None:
            require(type(case.get('skip')) is int and case['skip'] == skip, 'wrong publication cut')
        durable = case.get('durable')
        require(isinstance(durable, list) and len(durable) == 2 and all(isinstance(s, dict) for s in durable),
                'missing active and pending retained records')
        require(tuple(s.get('state') for s in durable) == states, 'retained states contradict the scheduled failure cut')
        for status in durable:
            identity(status)
        require(durable[0]['id'] != durable[1]['id'] and durable[0]['lineage'] == durable[1]['lineage'],
                'active and pending records alias or belong to different volumes')
        observations = case.get('observations')
        require(isinstance(observations, list) and len(observations) == len(expected), 'missing scheduled failure observations')
        for actual, (index, phase, pending, requested) in zip(observations, expected):
            check_operation(catalog, live_operation(durable[index], actual))
            require((actual['phase'], actual['pending'], actual['requested']) == (phase, pending, requested),
                    'live scheduling evidence does not establish the selected failure boundary')
        require(version(case.get('previous_version')) and version(case.get('file_version')),
                'missing actual native file versions')
        commits = [(i, s) for i, s in enumerate(durable) if s['state'] == 'committed']
        receipts = case.get('completions')
        require(isinstance(receipts, list) and len(receipts) == len(commits), 'missing or extra recovery completion receipts')
        for (index, status), completion in zip(commits, receipts):
            require(isinstance(completion, dict), 'invalid recovered completion')
            check_operation(catalog, completion)
            require((completion.get('operation_id'), completion.get('service_instance')) == identity(status)
                    and completion.get('state') == 'succeeded', 'recovery receipt belongs to unrelated work')
            payload = (b'first', b'second')[index]
            receipt = completion['receipt']
            require(receipt['sha256'] == hashlib.sha256(payload).hexdigest() and receipt['size'] == len(payload)
                    and receipt['version'] == f"v_{status['terminal']:016x}"
                    and receipt['previous_version'] == case['previous_version'], 'recovered receipt contradicts file arguments')
        expected_version = f"v_{commits[-1][1]['terminal']:016x}" if commits else case['previous_version']
        require(case['file_version'] == expected_version and type(case.get('file_size')) is int
                and case['file_size'] == len(content) and case.get('file_sha256') == hashlib.sha256(content).hexdigest(),
                'independent bytes/version contradict scheduled recovery')
        flags = ['reclaimed'] if not name.startswith('io_') else ['pending_abandoned', 'reconciled_after_restart']
        if name.startswith('late_') or name == 'lost_stop':
            flags.append('cancel_only')
        if name.startswith('lost_'):
            flags += ['discarded_reply', 'stale_reply_rejected']
            require(case.get('terminal_reply_ready') is (name == 'lost_result'), 'missing terminal-response readiness boundary')
        if name.startswith('restart_'):
            flags += ['restart_drained_io', 'owner_progress']
            require(all(type(case.get(f)) is int and case[f] > 0 for f in ('old_service_pid', 'new_service_pid'))
                    and case['old_service_pid'] != case['new_service_pid'], 'service process was not replaced')
        if name.startswith('io_'):
            require(case.get('query_error') == 'Uncertain', 'failed settlement claimed a known effect before recovery')
            uncertain = uncertain_operation(durable[0], True)
            check_operation(catalog, uncertain)
            require(uncertain['state'] == 'reconciling' and uncertain['effect'] == 'unknown', 'fault mapped to false success')
        if name == 'pressure':
            flags += ['staging_full', 'retained_full', 'undrained_client', 'staging_retained', 'owner_progress']
        require(all(case.get(f) is True for f in flags), 'missing scheduled failure authority/progress evidence')
    return len(found)
