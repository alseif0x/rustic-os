# SPDX-License-Identifier: Apache-2.0
"""Stops at the real header/final-flush boundary cannot roll back scheduled work."""
from ..scheduling_cases import prepare, settled, observation
from ..activity_cases import act
from ..authority_cases import cleanup
from ..cases import counters, pid
from ..oracle import snapshot
from .evidence import queue_pair, completions, proof, reboot


def verify(session, owned_disk, temporary, image, mount):
    cases, base = [], None
    for suffix, skip in (('header', 15), ('flush', 16)):
        name = 'scheduled_failure_late_' + suffix
        with owned_disk(temporary / (name + '.raw'), True, evidence_name=name) as data:
            with session(image, data, name) as uart:
                node, admissions = prepare(uart)
                if base is None:
                    base = (snapshot(data)[0], node, admissions)
                baseline = counters(uart)
                child = pid(uart, 'admission-session hello other 8')
                observations = queue_pair(uart, admissions, skip)
                stop = act(uart, child, 'request-cancel', admissions[0]['id'])
                if (stop['value'], stop['other'], stop['control_denied']) != (3, 1, 1):
                    raise AssertionError('late CANCEL-only stop did not acknowledge settlement')
                observations.append(observation(uart, f"admission-activity {admissions[0]['id']}", 'settling', 1, 1))
                uart.command('io-status', 'held=1')
                finals = [settled(uart, a) for a in admissions]
                if [a['state'] for a in finals] != ['committed', 'cancelled']:
                    raise AssertionError('late stop rolled back publication or bypassed queued version guard')
                receipts = completions(uart, finals)
                cleanup(uart, child)
                if counters(uart) != baseline:
                    raise AssertionError('late scheduled stop leaked resources')
            reboot(session, mount, data, name, finals, b'first')
            disk = proof(data, node, finals, b'first')
        cases.append(dict(case=name, verified=True, reboot_verified=True, no_replay=True,
                          skip=skip, observations=observations, durable=finals, completions=receipts,
                          cancel_only=True, reclaimed=True, **disk))
    return cases, base
