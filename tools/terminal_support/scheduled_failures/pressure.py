# SPDX-License-Identifier: Apache-2.0
"""Full retention/staging plus an unread client cannot starve scheduled stop/status."""
from ..scheduling_cases import prepare, settled, observation
from ..activity_cases import act, held
from ..authority_cases import actor, cleanup, fence
from ..cases import counters, pid
from .evidence import completions, proof, reboot


def verify(session, owned_disk, temporary, image, mount):
    name = 'scheduled_failure_pressure'
    with owned_disk(temporary / (name + '.raw'), True, evidence_name=name) as data:
        with session(image, data, name) as uart:
            node, admissions = prepare(uart)
            first, second = admissions
            baseline = counters(uart)
            executor = pid(uart, 'admission-session hello other 7')
            stalled = pid(uart, 'admission-session hello other 3')
            for child in (executor, stalled):
                actor(uart, child, 'read'); actor(uart, child, 'stage')
            uart.command('write hello staging-full', 'error: Busy')
            filled = actor(uart, stalled, 'flood')
            if filled['value'] < 4 or filled['other'] == 0:
                raise AssertionError('stalled client did not fill its actual request queue')
            uart.command('hold-io 0 400', 'diagnostic armed')
            observations = [observation(uart, f"schedule-admission {first['id']}", 'queued', 0)]
            held(uart)
            observations.append(observation(uart, f"admission-activity {first['id']}", 'running', 1))
            queued = act(uart, executor, 'schedule', second['id'])
            if (queued['value'], queued['other'], queued['control_denied']) != (4, 0, 0):
                raise AssertionError('queue pressure prevented pending admission')
            observations.append(observation(uart, f"admission-activity {second['id']}", 'queued', 0))
            uart.command('write hello borrowed-storage', 'error: Busy')
            observations.append(observation(uart, f"request-cancel {second['id']}", 'queued', 0, 1))
            observations.append(observation(uart, f"request-cancel {first['id']}", 'stopping', 1, 1))
            uart.command('echo owner-during-scheduled-pressure', 'owner-during-scheduled-pressure')
            uart.command('io-status', 'held=1')
            finals = [settled(uart, a) for a in admissions]
            if [a['state'] for a in finals] != ['cancelled', 'cancelled']:
                raise AssertionError('pressure prevented accepted stops from settling')
            if actor(uart, stalled, 'drain')['value'] < 1:
                raise AssertionError('undrained client lost its retained replies')
            # Each original staging buffer still exists until the owner explicitly
            # revokes it; a queued publication must not consume or replace it.
            for child in (executor, stalled):
                fence(uart, child, 'access=fenced members=1 discarded_staging=1 effects=settled')
            cleanup(uart, executor, stalled)
            if counters(uart) != baseline:
                raise AssertionError('combined pressure leaked process/channel/I/O resources')
            receipts = completions(uart, finals)
        reboot(session, mount, data, name, finals, b'before')
        disk = proof(data, node, finals, b'before')
    return [dict(case=name, verified=True, reboot_verified=True, no_replay=True,
                 observations=observations, durable=finals, completions=receipts,
                 staging_full=True, retained_full=True, undrained_client=True,
                 staging_retained=True, owner_progress=True, reclaimed=True, **disk)]
