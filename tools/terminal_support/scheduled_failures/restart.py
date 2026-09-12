# SPDX-License-Identifier: Apache-2.0
"""Restart the actual file-service process while active and pending work coexist."""
import re
from ..scheduling_cases import prepare
from ..admission_cases import status
from ..cases import counters
from ..management_cases import start_restart, wait_job
from .evidence import queue_pair, completions, proof, reboot


def service_pid(uart):
    return int(re.search(r'files pid=(\d+)', uart.command('services'))[1])


def verify(session, owned_disk, temporary, image, mount):
    cases = []
    for suffix, skip in (('data', 0), ('flush', 16)):
        name = 'scheduled_failure_restart_' + suffix
        content = b'first' if skip else b'before'
        with owned_disk(temporary / (name + '.raw'), True, evidence_name=name) as data:
            with session(image, data, name) as uart:
                node, admissions = prepare(uart)
                baseline = counters(uart)
                old_pid = service_pid(uart)
                observations = queue_pair(uart, admissions, skip)
                job = start_restart(uart)
                progress = uart.command(f'job-status {job}', 'pending_io=1')
                if 'phase=2' not in progress:
                    raise AssertionError('restart did not retain the pending device command')
                uart.command('echo owner-during-scheduled-restart', 'owner-during-scheduled-restart')
                wait_job(uart, job)
                new_pid = service_pid(uart)
                if new_pid == old_pid or counters(uart) != baseline:
                    raise AssertionError('restart did not replace/reclaim the original service')
                finals = [status(uart.command(f"admission {a['id']}")) for a in admissions]
                expected = ['committed' if skip else 'admitted', 'admitted']
                if [a['state'] for a in finals] != expected:
                    raise AssertionError('service restart replayed pending work or misstated the active effect')
                for final in finals:
                    uart.command(f"admission-activity {final['id']}", 'error: Unavailable')
                receipts = completions(uart, finals)
            reboot(session, mount, data, name, finals, content)
            disk = proof(data, node, finals, content)
        cases.append(dict(case=name, verified=True, reboot_verified=True, no_replay=True,
                          skip=skip, observations=observations, durable=finals, completions=receipts,
                          old_service_pid=old_pid, new_service_pid=new_pid,
                          restart_drained_io=True, owner_progress=True, reclaimed=True, **disk))
    return cases
