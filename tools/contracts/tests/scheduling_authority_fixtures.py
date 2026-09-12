# SPDX-License-Identifier: Apache-2.0
"""Synthetic examples for adversarial validator tests, never native evidence."""
import copy
import hashlib
from .scheduled_failure_fixtures import failure_cases


def authority_cases():
    result = []
    groups = (
        ('read_write', ((1, 'hello', False, False, 17, 17, 17, 17), (2, 'hello', False, False, 17, 17, 17, 17))),
        ('inspect_all', ((4, 'hello', False, False, 0, 17, 0, 17), (7, 'hello', False, False, 0, 0, 0, 17))),
        ('scope_subject', ((15, 'other', False, False, 27, 27, 27, 27), (15, 'hello', True, False, 27, 27, 27, 27))),
        ('revoked_cancel', ((15, 'hello', False, True, 18, 18, 18, 18), (8, 'hello', False, False, 17, 17, 17, 0))),
    )
    def reply(status, value=0, other=0, pending=0):
        return dict(status=status, value=value, other=other, control_denied=pending, version=0)
    for name, specs in groups:
        case = copy.deepcopy(failure_cases()[0])
        case['case'] = 'scheduling_authority_' + name
        durable = case['durable']
        def observation(index, phase, pending, requested=0):
            return dict(id=durable[index]['id'], instance=durable[index]['instance'],
                        phase=phase, pending=pending, requested=requested)
        case['observations'] = [observation(0, 'queued', 0), observation(0, 'running', 1), observation(1, 'queued', 0)]
        actors = []
        for i, (rights, scope, private, revoked, initial, schedule, activity, cancel) in enumerate(specs):
            actor = dict(pid=i+4, rights=rights, scope=scope, private=private, revoked=revoked,
                         subject=i+4 if private else 1, initial=reply(initial, 0 if initial else 1), attempts=[])
            for index, action, error in ((1, 'schedule', schedule), (0, 'activity', activity),
                                        (1, 'activity', activity), (1, 'request-cancel', cancel), (0, 'request-cancel', cancel)):
                stop = int(action == 'request-cancel')
                value = 4 if index else 2 if stop else 1
                actor['attempts'].append(dict(index=index, action=action,
                    **reply(error, 0 if error else value, 0 if error else stop, 0 if error else int(index == 0))))
            actors.append(actor)
            stopped = cancel == 0
            case['observations'] += [observation(0, 'stopping' if stopped else 'running', 1, int(stopped)),
                                     observation(1, 'queued', 0, int(stopped))]
        case['actors'] = actors
        if name == 'revoked_cancel':
            durable[0]['state'] = 'cancelled'
            case.update(completions=[], file_version=case['previous_version'], file_size=6,
                        file_sha256=hashlib.sha256(b'before').hexdigest())
        result.append(case)
    human = copy.deepcopy(result[0])
    human['case'] = 'scheduling_authority_human_edit'
    human['admitted'] = dict(human['durable'][0], state='admitted', terminal=0)
    human['durable'] = [dict(human['durable'][0], state='cancelled')]
    human['observations'] = human['observations'][:1]
    human.update(conflict='Version', conflict_unchanged=True, completions=[], file_version='v_0000000000000004',
                 file_size=10, file_sha256=hashlib.sha256(b'human-edit').hexdigest())
    result.append(human)
    return result
