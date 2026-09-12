# SPDX-License-Identifier: Apache-2.0
"""Unread cancellation acknowledgement and terminal query response under scheduling."""
from ..scheduling_cases import prepare, settled, observation
from ..activity_cases import act
from ..authority_cases import actor_result, cleanup
from ..cases import counters, pid
from ..oracle import snapshot
from .evidence import queue_pair, completions, proof, reboot, retained


def verify(session, owned_disk, temporary, image, mount):
    cases = []
    for kind in ('stop', 'result'):
        name = 'scheduled_failure_lost_' + kind
        content = b'second' if kind == 'stop' else b'first'
        with owned_disk(temporary / (name + '.raw'), True, evidence_name=name) as data:
            with session(image, data, name) as uart:
                node, admissions = prepare(uart)
                baseline = counters(uart)
                child = pid(uart, f"admission-session hello other {8 if kind == 'stop' else 4}")
                observations = queue_pair(uart, admissions, 0)
                if kind == 'stop':
                    uart.command(f"act-admission {child} lost-stop {admissions[0]['id']}", 'actor state=pending')
                    actor_result(uart, child)
                    observations.append(observation(uart, f"admission-activity {admissions[0]['id']}", 'stopping', 1, 1))
                finals = [settled(uart, a) for a in admissions]
                expected = ['cancelled', 'committed'] if kind == 'stop' else ['committed', 'cancelled']
                if [a['state'] for a in finals] != expected:
                    raise AssertionError('lost response changed scheduled effects or pending progress')
                receipts = completions(uart, finals)
                before = snapshot(data)[0]
                if kind == 'result':
                    # GET has a terminal record at this point. The actor waits for
                    # reply readiness, but never receives or decodes that response.
                    act(uart, child, 'lost-result', admissions[0]['id'])
                act(uart, child, 'get' if kind == 'result' else 'activity', admissions[0]['id'], 1)
                retained(uart, finals)
                cleanup(uart, child)
                if counters(uart) != baseline or snapshot(data)[0] != before:
                    raise AssertionError('lost response replayed disk work or leaked a client')
            reboot(session, mount, data, name, finals, content)
            disk = proof(data, node, finals, content)
        cases.append(dict(case=name, verified=True, reboot_verified=True, no_replay=True,
                          observations=observations, durable=finals, completions=receipts,
                          discarded_reply=True, stale_reply_rejected=True, reclaimed=True,
                          terminal_reply_ready=kind == 'result', cancel_only=kind == 'stop', **disk))
    return cases
