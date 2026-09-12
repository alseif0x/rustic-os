# SPDX-License-Identifier: Apache-2.0
"""Real EIO before/after publication discards volatile queued work and requires recovery."""
from ..admission_cases import status
from ..scheduling_cases import observation
from ..management_cases import start_restart, wait_job
from .evidence import queue_pair, await_uncertain, completions, proof, reboot


def verify(session, owned_disk, temporary, mount, base):
    prefix, node, admissions = base
    cases = []
    for suffix, skip in (('data', 0), ('flush', 16)):
        name = 'scheduled_failure_io_' + suffix
        content = b'first' if skip else b'before'
        with owned_disk(temporary / (name + '.raw'), True, evidence_name=name) as data:
            with data.open('r+b') as stream:
                stream.write(prefix)
            with session(mount, data, name, skip) as uart:
                observations = queue_pair(uart, admissions, skip)
                observations.append(observation(uart, f"request-cancel {admissions[0]['id']}",
                                                'settling' if skip else 'stopping', 1, 1))
                query_error = await_uncertain(uart, admissions[0])
                uart.command('mem', 'pending_io=0')
                wait_job(uart, start_restart(uart))
                finals = [status(uart.command(f"admission {a['id']}")) for a in admissions]
                if [a['state'] for a in finals] != ['committed' if skip else 'admitted', 'admitted']:
                    raise AssertionError('fault fabricated cancellation or executed the abandoned queue')
                for final in finals:
                    uart.command(f"admission-activity {final['id']}", 'error: Unavailable')
                receipts = completions(uart, finals)
            reboot(session, mount, data, name, finals, content)
            disk = proof(data, node, finals, content)
        cases.append(dict(case=name, verified=True, reboot_verified=True, no_replay=True,
                          skip=skip, observations=observations, durable=finals, completions=receipts,
                          query_error=query_error, reconciled_after_restart=True,
                          pending_abandoned=True, **disk))
    return cases
