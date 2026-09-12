# SPDX-License-Identifier: Apache-2.0
"""Synthetic coherent replies. These are test inputs, never native evidence."""


def add_observations(case):
    def retained(index, state):
        record = case['durable'][index]
        return dict(profile=1, id=record['id'], instance=record['instance'], kind='retained',
                    state=state, terminal=0 if state == 'admitted' else record['terminal'])
    def active(index, phase, pending, requested):
        record = case['durable'][index]
        return dict(profile=1, id=record['id'], instance=record['instance'], kind='active',
                    phase=phase, pending=pending, requested=requested)
    if case['case'] == 'scheduled_queue':
        case['coherent_observations'] = [retained(0, 'admitted'), retained(1, 'admitted'),
            active(0, 'running', 1, 0), retained(1, 'admitted'), active(1, 'queued', 0, 0),
            active(1, 'queued', 0, 1), retained(0, 'committed'), retained(1, 'cancelled'),
            retained(0, 'committed'), retained(1, 'cancelled')]
        case['observation_clients'] = [dict(index=i, result=dict(status=0, value=v, other=o,
            control_denied=p, version=0)) for i, v, o, p in (
                (0, 1, 0, 0), (2, 17, 0, 1), (4, 20, 0, 0), (5, 20, 1, 0),
                (6, 3, case['durable'][0]['terminal'], 0), (7, 2, case['durable'][1]['terminal'], 0))]
        case['observation_denied'] = dict(status=17, value=0, other=0, control_denied=0, version=0)
        case['observation_read_only'] = True
    elif case['case'] == 'scheduled_restart':
        case['coherent_observations'] = [retained(0, 'admitted'), retained(1, 'admitted'), retained(1, 'committed')]
    elif case['case'] == 'scheduled_revoked':
        case['observation_revoked'] = dict(status=18, value=0, other=0, control_denied=0, version=0)
