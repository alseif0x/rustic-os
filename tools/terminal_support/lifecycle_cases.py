# SPDX-License-Identifier: Apache-2.0
"""Disposable native lifecycle cases; all mutation is through the file service."""
from . import lifecycle_observations as logical
from .admission_cases import status
from .activity_cases import act, held
from .authority_cases import cleanup
from .cases import counters, pid
from .oracle import snapshot
from .scheduling_cases import settled


def exercise(uart, data, workspace, resource):
    baseline = counters(uart)
    file_id = int(resource.rsplit('_', 1)[1], 16)
    original_files = snapshot(data)[1]['files']
    cases = []
    for case, skip in [('queued', 0), ('active', 0), ('late_lost', 15)]:
        uart.command('rotate-receipts')
        old = snapshot(data)[1]
        version = old['nodes'][file_id]['version']
        def admit(key):
            return status(uart.command(f'admit-ref {workspace} {resource} v_{version:016x} e_{old["epoch"]:016x} k_{key:016x} "Hello from native Rust"'))
        first = admit(0x8100)
        second = admit(0x8101) if case == 'queued' else first
        executor = pid(uart, 'admission-session hello /config/owner-policy 7')
        canceller = pid(uart, 'admission-session hello /config/owner-policy 8')
        item = dict(case=case, skip=skip, prepared=logical.inspect(uart, second, 'prepared', executor))
        item['no_cancel_denied'] = logical.denied(uart, executor, 'cancel', second)
        item['cancel_only_denied'] = logical.denied(uart, canceller, 'inspect', second)
        uart.command(f'hold-io {skip} 500', 'diagnostic armed')
        act(uart, executor, 'schedule', first['id'])
        held(uart)
        phase = 'reconciling' if skip == 15 else 'running'
        item['before'] = logical.inspect(uart, first, phase, executor, stop=False)
        if case == 'late_lost':
            item['lost_reply'] = act(uart, canceller, 'lost-cancel', second['id'])
            # A separate authorized observation, never the discarded ACK, proves acceptance.
            item['stopped'] = logical.inspect(uart, first, phase, executor, stop=True)
            item['repeat'] = logical.cancel(uart, second, 'already_requested')
        else:
            item['accepted'] = logical.cancel(uart, second, 'requested', canceller)
            item['repeat'] = logical.cancel(uart, second, 'already_requested')
            item['stopped'] = logical.inspect(uart, second, 'queued' if case == 'queued' else phase, executor, stop=True)
        uart.command('io-status', 'held=1')
        final_first = settled(uart, first)
        final_second = settled(uart, second) if case == 'queued' else final_first
        expected_first = 'committed' if case != 'active' else 'cancelled'
        expected_second = 'cancelled' if case != 'late_lost' else 'committed'
        if (final_first['state'], final_second['state']) != (expected_first, expected_second):
            raise AssertionError('stop acceptance was confused with the committed effect boundary')
        item['final'] = [final_first, final_second] if case == 'queued' else [final_first]
        item['terminal'] = []
        for result in item['final']:
            committed = result['state'] == 'committed'
            completion = f"op_{result['lineage']}_{result['terminal']:016x}" if committed else None
            item['terminal'].append(logical.inspect(uart, result, 'succeeded' if committed else 'cancelled', executor, completion=completion))
        item['too_late'] = logical.cancel(uart, second, 'too_late')
        # A discarded reply still occupies that binding; never consume it as a later reply.
        if case != 'late_lost':
            item['too_late_client'] = logical.cancel(uart, second, 'too_late', canceller)
        cleanup(uart, executor, canceller)
        state = snapshot(data)[1]
        if state['files'] != original_files or counters(uart) != baseline:
            raise AssertionError('lifecycle cases changed file contents or leaked resources')
        for result in item['final']:
            records = [r for r in state['records'] if r.get('admission') == result['number']]
            cause = None if result['state'] == 'committed' else 'requested'
            if len(records) != 1 or records[0]['terminal'] != result['terminal'] or records[0]['state'] != result['state'] or records[0]['prevention'] != cause:
                raise AssertionError('independent disk disagrees with logical terminal result')
        expected_version = final_first['terminal'] if final_first['state'] == 'committed' else version
        if state['nodes'][file_id]['version'] != expected_version:
            raise AssertionError('prevented work changed the file version')
        item['disk'] = dict(previous_version=version, file_version=state['nodes'][file_id]['version'],
                            records=[{k: r[k] for k in ('admission', 'state', 'terminal', 'prevention', 'committed')} for r in state['records']])
        item['disk_sha256'] = state['selected_sha256']
        cases.append(item)
    uart.command('rotate-receipts')
    return cases
