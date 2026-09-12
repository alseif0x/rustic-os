# SPDX-License-Identifier: Apache-2.0
"""Real active/queued IPC must recheck rights, object scope, subject and revocation."""
from ..activity_cases import act
from ..authority_cases import actor, cleanup, fence
from ..cases import counters, pid
from ..scheduling_cases import prepare, settled, observation
from ..scheduled_failures.evidence import queue_pair, completions, proof, reboot

# Full rights on foreign scope/subject distinguish identity denial from missing rights.
GROUPS = (
    ('read_write', ((1, 'hello', False, False), (2, 'hello', False, False))),
    ('inspect_all', ((4, 'hello', False, False), (7, 'hello', False, False))),
    ('scope_subject', ((15, 'other', False, False), (15, 'hello', True, False))),
    ('revoked_cancel', ((15, 'hello', False, True), (8, 'hello', False, False))),
)


def failure(rights, scope, private, revoked, required):
    if revoked: return 18
    if rights & required != required: return 17
    if scope != 'hello' or private: return 27
    return 0


def verify(session, owned_disk, temporary, image, mount):
    cases = []
    for suffix, specs in GROUPS:
        name = 'scheduling_authority_' + suffix
        stopped = suffix == 'revoked_cancel'
        with owned_disk(temporary / (name + '.raw'), True, evidence_name=name) as data:
            with session(image, data, name) as uart:
                node, admissions = prepare(uart)
                baseline = counters(uart)
                children, actors = [], []
                for rights, scope, private, revoked in specs:
                    child = pid(uart, f'admission-session {scope} hello {rights}' + (' private' if private else ''))
                    children.append(child)
                    uart.command(f'permissions {child}', f'rights={rights}')
                    if rights & 1:
                        actor(uart, child, 'read')
                    if revoked:
                        fence(uart, child, 'access=fenced members=1')
                        uart.command(f'permissions {child}', 'rights=0')
                    expected = failure(rights, scope, private, revoked, 4)
                    initial = act(uart, child, 'get', admissions[0]['id'], expected)
                    if private and child <= 1:
                        raise AssertionError('private diagnostic subject aliases the owner')
                    actors.append(dict(pid=child, rights=rights, scope=scope, private=private,
                                       revoked=revoked, subject=child if private else 1,
                                       initial=initial, attempts=[]))
                observations = queue_pair(uart, admissions, 0)
                for evidence, spec in zip(actors, specs):
                    rights, scope, private, revoked = spec
                    child = evidence['pid']
                    # Duplicate schedule requires both rights but must neither steal
                    # ownership nor create another ticket. The retained pair is full.
                    for index, action, required in ((1, 'schedule', 6), (0, 'activity', 4),
                                                    (1, 'activity', 4), (1, 'request-cancel', 8),
                                                    (0, 'request-cancel', 8)):
                        expected = failure(*spec, required)
                        result = act(uart, child, action, admissions[index]['id'], expected)
                        if expected and any(result[key] for key in ('value', 'other', 'control_denied', 'version')):
                            raise AssertionError('denial leaked live operation state')
                        evidence['attempts'].append(dict(index=index, action=action, **result))
                    accepted = not failure(*spec, 8)
                    observations += [observation(uart, f"admission-activity {a['id']}",
                                                 'stopping' if accepted and i == 0 else 'running' if i == 0 else 'queued',
                                                 int(i == 0), int(accepted))
                                     for i, a in enumerate(admissions)]
                uart.command('echo owner-after-scheduling-denials', 'owner-after-scheduling-denials')
                uart.command('io-status', 'held=1')
                finals = [settled(uart, a) for a in admissions]
                if [f['state'] for f in finals] != (['cancelled', 'cancelled'] if stopped else ['committed', 'cancelled']):
                    raise AssertionError('denied control changed authorized settlement')
                receipts = completions(uart, finals)
                cleanup(uart, *children)
                if counters(uart) != baseline:
                    raise AssertionError('authority fixture leaked bindings or processes')
            content = b'before' if stopped else b'first'
            reboot(session, mount, data, name, finals, content)
            disk = proof(data, node, finals, content)
        cases.append(dict(case=name, verified=True, reboot_verified=True, no_replay=True,
                          actors=actors, observations=observations, durable=finals, completions=receipts,
                          owner_progress=True, reclaimed=True, **disk))
    return cases
