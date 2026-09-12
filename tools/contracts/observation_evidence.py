# SPDX-License-Identifier: Apache-2.0
"""A single reply cannot fabricate a terminal fact or resume recovered work."""
from .validation import require


def exact(actual, expected, message):
    require(isinstance(actual, dict) and actual.keys() == expected.keys()
            and all(type(actual[k]) is type(v) and actual[k] == v for k, v in expected.items()), message)


def report(actual, status=0, value=0, other=0, pending=0):
    exact(actual, dict(status=status, value=value, other=other, control_denied=pending, version=0),
          'observation client result disagrees or leaks data in a denial')


def observations(case, expected):
    actual = case.get('coherent_observations')
    require(isinstance(actual, list) and len(actual) == len(expected), 'missing coherent observations')
    for view, (index, kind, state, pending, requested) in zip(actual, expected):
        retained = case['durable'][index]
        value = dict(profile=1, id=retained['id'], instance=retained['instance'], kind=kind)
        if kind == 'retained':
            value.update(state=state, terminal=0 if state == 'admitted' else retained['terminal'])
        else:
            value.update(phase=state, pending=pending, requested=requested)
        exact(view, value, 'coherent observation lost its stable identity, profile or execution boundary')
    return actual


def check_observations(cases):
    # Called after the scheduling validator verifies inventory, identity, disk
    # receipts and restart/authority evidence. This checks the new single replies.
    queue = next(c for c in cases if c['case'] == 'scheduled_queue')
    expected = ((0, 'retained', 'admitted', 0, 0), (1, 'retained', 'admitted', 0, 0),
                (0, 'active', 'running', 1, 0), (1, 'retained', 'admitted', 0, 0),
                (1, 'active', 'queued', 0, 0), (1, 'active', 'queued', 0, 1),
                (0, 'retained', 'committed', 0, 0), (1, 'retained', 'cancelled', 0, 0),
                (0, 'retained', 'committed', 0, 0), (1, 'retained', 'cancelled', 0, 0))
    views = observations(queue, expected)
    require(queue.get('observation_read_only') is True, 'observation changed storage')
    clients = queue.get('observation_clients')
    indices = (0, 2, 4, 5, 6, 7)
    require(isinstance(clients, list) and len(clients) == len(indices), 'missing deterministic observations')
    for client, index in zip(clients, indices):
        require(isinstance(client, dict) and set(client) == {'index', 'result'}
                and type(client['index']) is int and client['index'] == index,
                'deterministic observation belongs to another request')
        view = views[index]
        if view['kind'] == 'retained':
            report(client['result'], value={'admitted': 1, 'cancelled': 2, 'committed': 3}[view['state']],
                   other=view['terminal'])
        else:
            report(client['result'], value=0x10 | {'running': 1, 'queued': 4}[view['phase']],
                   other=view['requested'], pending=view['pending'])
    report(queue.get('observation_denied'), status=17)
    restart = next(c for c in cases if c['case'] == 'scheduled_restart')
    observations(restart, ((0, 'retained', 'admitted', 0, 0), (1, 'retained', 'admitted', 0, 0),
                           (1, 'retained', 'committed', 0, 0)))
    revoked = next(c for c in cases if c['case'] == 'scheduled_revoked')
    report(revoked.get('observation_revoked'), status=18)
    return 13
