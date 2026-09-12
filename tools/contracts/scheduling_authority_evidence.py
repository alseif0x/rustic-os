# SPDX-License-Identifier: Apache-2.0
"""Validate native scheduling denials and preservation of a real intervening edit."""
import hashlib
import re
from .validation import require
from .activity_conformance import identity, live_operation, check_operation

PREFIX = 'scheduling_authority_'
ACTORS = {
    'read_write': ((1, 'hello', False, False, 17, 17, 17, 17),
                   (2, 'hello', False, False, 17, 17, 17, 17)),
    'inspect_all': ((4, 'hello', False, False, 0, 17, 0, 17),
                    (7, 'hello', False, False, 0, 0, 0, 17)),
    'scope_subject': ((15, 'other', False, False, 27, 27, 27, 27),
                      (15, 'hello', True, False, 27, 27, 27, 27)),
    'revoked_cancel': ((15, 'hello', False, True, 18, 18, 18, 18),
                       (8, 'hello', False, False, 17, 17, 17, 0)),
}


def report(actual, status, value=0, other=0, pending=0):
    require(isinstance(actual, dict) and all(type(actual.get(k)) is int and actual[k] == v
            for k, v in zip(('status', 'value', 'other', 'control_denied', 'version'),
                            (status, value, other, pending, 0))),
            'actor reply omitted its denial or leaked/changed operation state')


def version(value):
    require(isinstance(value, str) and re.fullmatch(r'v_[0-9a-f]{16}', value)
            and int(value[2:], 16) > 0, 'invalid file version')
    return int(value[2:], 16)


def observe(catalog, durable, observations, expected):
    require(isinstance(observations, list) and len(observations) == len(expected),
            'missing authority observations')
    for actual, (index, phase, pending, requested) in zip(observations, expected):
        check_operation(catalog, live_operation(durable[index], actual))
        require((actual['phase'], actual['pending'], actual['requested']) == (phase, pending, requested),
                'denied control changed the live operation or lost its I/O boundary')


def check_authority(catalog, cases):
    found = [c for c in cases if isinstance(c, dict) and str(c.get('case', '')).startswith(PREFIX)]
    require(len(found) == 5 and {c['case'] for c in found} == {PREFIX + n for n in (*ACTORS, 'human_edit')},
            'missing, duplicate or unknown scheduling authority group')
    for case in found:
        name = case['case'][len(PREFIX):]
        require(all(case.get(k) is True for k in ('verified', 'reboot_verified', 'no_replay', 'reclaimed')),
                'unverified authority or replay/cleanup evidence')
        require(isinstance(case.get('sha256'), str) and re.fullmatch(r'[0-9a-f]{64}', case['sha256']),
                'missing independent disk identity')
        human = name == 'human_edit'
        stopped = name == 'revoked_cancel'
        states = ('cancelled',) if human else ('cancelled' if stopped else 'committed', 'cancelled')
        durable = case.get('durable')
        require(isinstance(durable, list) and len(durable) == len(states)
                and all(isinstance(s, dict) for s in durable), 'missing retained authority outcomes')
        require(tuple(s.get('state') for s in durable) == states, 'authority cut changed the retained outcome')
        for s in durable: identity(s)
        require(len({s['id'] for s in durable}) == len(durable)
                and len({s['lineage'] for s in durable}) == 1, 'authority tickets alias or cross volumes')
        previous, current = version(case.get('previous_version')), version(case.get('file_version'))
        if human:
            admitted = case.get('admitted')
            require(isinstance(admitted, dict), 'missing original admitted work')
            identity(admitted)
            require(admitted['state'] == 'admitted' and all(admitted[k] == durable[0][k]
                    for k in ('id', 'lineage', 'number', 'instance')), 'stale prevention changed admission identity')
            require(case.get('conflict') == 'Version' and case.get('conflict_unchanged') is True
                    and previous < current < durable[0]['terminal'], 'missing intervening edit or preserved version conflict')
            observe(catalog, durable, case.get('observations'), ((0, 'queued', 0, 0),))
            content = b'human-edit'
            require(not case.get('completions'), 'stale candidate fabricated a successful receipt')
        else:
            require(case.get('owner_progress') is True, 'missing owner progress under authority denial')
            actors = case.get('actors')
            require(isinstance(actors, list) and len(actors) == 2 and all(isinstance(a, dict) for a in actors),
                    'missing independent actor bindings')
            require(all(type(a.get('pid')) is int and a['pid'] > 1 for a in actors)
                    and actors[0]['pid'] != actors[1]['pid'], 'actor identities alias the owner or each other')
            expected_obs = [(0, 'queued', 0, 0), (0, 'running', 1, 0), (1, 'queued', 0, 0)]
            for actor, spec in zip(actors, ACTORS[name]):
                rights, scope, private, revoked, initial, schedule, activity, cancel = spec
                require(type(actor.get('rights')) is int and actor['rights'] == rights
                        and actor.get('scope') == scope and actor.get('private') is private
                        and actor.get('revoked') is revoked and type(actor.get('subject')) is int
                        and actor['subject'] == (actor['pid'] if private else 1), 'wrong tested rights, scope or subject')
                report(actor.get('initial'), initial, 1 if not initial else 0)
                attempts = actor.get('attempts')
                require(isinstance(attempts, list) and len(attempts) == 5, 'incomplete active/pending denial matrix')
                for reply, (index, action, error) in zip(attempts, ((1, 'schedule', schedule),
                        (0, 'activity', activity), (1, 'activity', activity),
                        (1, 'request-cancel', cancel), (0, 'request-cancel', cancel))):
                    require(isinstance(reply, dict) and type(reply.get('index')) is int
                            and reply['index'] == index and reply.get('action') == action,
                            'missing, duplicate or reordered authority request')
                    stop = int(action == 'request-cancel')
                    report(reply, error, (4 if index else 2 if stop else 1) if not error else 0,
                           stop if not error else 0, int(index == 0) if not error else 0)
                accepted = int(cancel == 0)
                expected_obs += [(0, 'stopping' if accepted else 'running', 1, accepted),
                                 (1, 'queued', 0, accepted)]
            observe(catalog, durable, case.get('observations'), expected_obs)
            receipts = case.get('completions')
            require(isinstance(receipts, list) and len(receipts) == int(not stopped), 'wrong successful receipt inventory')
            content = b'before' if stopped else b'first'
            if stopped:
                require(current == previous, 'accepted early stop changed the file')
            else:
                completion = receipts[0]
                require(isinstance(completion, dict), 'invalid successful receipt')
                check_operation(catalog, completion)
                require((completion.get('operation_id'), completion.get('service_instance')) == identity(durable[0])
                        and completion.get('state') == 'succeeded', 'receipt belongs to unrelated work')
                receipt = completion['receipt']
                require(current == durable[0]['terminal'] and current > previous
                        and receipt['previous_version'] == case['previous_version']
                        and receipt['version'] == case['file_version'] and receipt['size'] == len(content)
                        and receipt['sha256'] == hashlib.sha256(content).hexdigest(), 'receipt contradicts independent file bytes/version')
        require(type(case.get('file_size')) is int and case['file_size'] == len(content)
                and case.get('file_sha256') == hashlib.sha256(content).hexdigest(), 'authority evidence contradicts actual content')
    return len(found)
