# SPDX-License-Identifier: Apache-2.0
"""One native utility owns selection, reading, admission and live control."""
import hashlib
from . import lifecycle_observations as logical
from .admission_cases import status
from .activity_cases import act, held
from .authority_cases import actor, cleanup, fence
from .cases import counters, pid
from .oracle import snapshot
from .scheduling_cases import settled

CONTENT = b'Single native client'


def exercise(uart, data, workspace, resource):
    baseline = counters(uart)
    file_id = int(resource.rsplit('_', 1)[1], 16)
    cases = []
    for kind in ('complete', 'cancel', 'revoke'):
        uart.command('rotate-receipts')
        before = snapshot(data)[1]
        old_version = before['nodes'][file_id]['version']
        client = pid(uart, 'admission-session hello /config/owner-policy 15')
        active = counters(uart)
        if active['processes'] != baseline['processes'] + 1 or active['channels'] != baseline['channels'] + 2:
            raise AssertionError('selected mission requires exactly one utility and its two channels')
        item = dict(case=kind, client=client, rights=15, selection=[])
        for action, method in [('select-get', 5), ('select-cancel', 6)]:
            selected = actor(uart, client, action)
            if selected != dict(status=0, value=1, other=method, control_denied=1, version=2):
                raise AssertionError('client did not select the reviewed available method')
            item['selection'].append(selected)
        for method in ('operations.get', 'operations.cancel'):
            uart.command(f'select-lifecycle {method}', f'lifecycle-selected method={method} availability=available')
        item['prepared'] = actor(uart, client, 'mission-prepare')
        prepared = item['prepared']
        if prepared['version'] != old_version or prepared['other'] != before['epoch'] or prepared['control_denied'] != len(before['nodes'][file_id]['content']):
            raise AssertionError('utility did not read the current version/epoch/content length')
        # The owner only observes the admission made by the utility. It neither
        # supplies the candidate's version nor prepares/schedules work for it.
        admission = status(uart.command(f'admission {workspace} e_{before["epoch"]:016x} k_0000000000008300'))
        if admission['number'] != prepared['value'] or admission['state'] != 'admitted':
            raise AssertionError('utility admission does not match retained retry identity')
        logical.inspect(uart, admission, 'prepared', client, selected=True)
        actor(uart, client, 'mission-prepare', 20)  # no automatic second candidate
        uart.command('hold-io 0 500', 'diagnostic armed')
        scheduled = act(uart, client, 'schedule', admission['id'])
        if scheduled != dict(status=0, value=4, other=0, control_denied=0, version=0):
            raise AssertionError('same-client scheduling did not acknowledge the queued ticket')
        held(uart)
        item['running'] = logical.inspect(uart, admission, 'running', client, stop=False, selected=True)
        if kind == 'cancel':
            item['ack'] = logical.cancel(uart, admission, 'requested', client, selected=True)
            logical.inspect(uart, admission, 'running', client, stop=True, selected=True)
        elif kind == 'revoke':
            fence(uart, client, 'access=fenced members=1')
            item['denied'] = [logical.denied(uart, client, action, admission, 18)
                              for action in ('inspect-selected', 'cancel-selected')]
        final = settled(uart, admission)
        committed = kind == 'complete'
        completion = f"op_{final['lineage']}_{final['terminal']:016x}" if committed else None
        state = 'succeeded' if committed else 'cancelled' if kind == 'cancel' else 'failed'
        failure = 'access_denied' if kind == 'revoke' else None
        item['terminal'] = logical.inspect(uart, final, state, None if kind == 'revoke' else client,
                                           failure=failure, completion=completion, selected=True)
        if kind != 'revoke':
            logical.cancel(uart, final, 'too_late', client, selected=True)
        if committed:
            item['readback'] = actor(uart, client, 'mission-verify')
            if item['readback'] != dict(status=0, value=len(CONTENT), other=before['epoch'], control_denied=1, version=final['terminal']):
                raise AssertionError('same client did not verify the committed bytes/hash/version')
        after = snapshot(data)[1]
        records = [r for r in after['records'] if r.get('admission') == admission['number']]
        cause = None if committed else 'requested' if kind == 'cancel' else 'authority_lost'
        if len(records) != 1 or records[0]['state'] != ('committed' if committed else 'cancelled') or records[0]['prevention'] != cause or records[0]['terminal'] != final['terminal'] or records[0]['committed'] != (final['terminal'] if committed else 0):
            raise AssertionError('selected mission disagrees with independent disk outcome')
        if records[0]['previous'] != old_version or records[0]['id'] != file_id or records[0]['content'] != CONTENT or records[0]['key'] != 0x8300 or records[0]['epoch'] != before['epoch'] or admission['instance'] != f"si_{after['lineage']}_{records[0]['instance']:016x}":
            raise AssertionError('persisted arguments or historical origin differ from utility admission')
        expected_files = dict(before['files'])
        key = next(k for k, v in before['files'].items() if k[1] == 'hello')
        if committed:
            expected_files[key] = CONTENT
        if after['files'] != expected_files or after['nodes'][file_id]['version'] != (final['terminal'] if committed else old_version):
            raise AssertionError('wrong file effect, rollback claim or foreign-workspace mutation')
        item['disk'] = dict(previous=old_version, version=after['nodes'][file_id]['version'],
                            epoch=before['epoch'], length=prepared['control_denied'],
                            sha256=hashlib.sha256(after['nodes'][file_id]['content']).hexdigest(),
                            record={k: records[0][k] for k in ('admission', 'state', 'terminal', 'prevention', 'committed')})
        cleanup(uart, client)
        if counters(uart) != baseline:
            raise AssertionError('selected mission leaked resources')
        item['resources'] = [baseline['processes'], active['processes'], baseline['channels'], active['channels']]
        cases.append(item)
        if committed:
            uart.command('write hello "Hello from native Rust"')
    uart.command('rotate-receipts')
    return cases
