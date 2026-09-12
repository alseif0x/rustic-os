# SPDX-License-Identifier: Apache-2.0
"""An actual shell edit invalidates admitted arguments before native scheduling."""
import hashlib
from ..admission_cases import status, check
from ..oracle import snapshot
from ..recovery_cases import stat
from ..scheduling_cases import prepare, observation, settled
from ..cases import counters


def verify(session, owned_disk, temporary, image, mount):
    name = 'scheduling_authority_human_edit'
    with owned_disk(temporary / (name + '.raw'), True, evidence_name=name) as data:
        with session(image, data, name) as uart:
            node, (admitted,) = prepare(uart, False)
            baseline = counters(uart)
            uart.command('write hello human-edit')
            edited = stat(uart, 'hello')
            if edited['version'] == node['version']:
                raise AssertionError('human edit did not change the native file version')
            before = snapshot(data)[0]
            uart.command(f"execute-admission {admitted['id']}", 'error: Version')
            if snapshot(data)[0] != before or status(uart.command(f"admission {admitted['id']}")) != admitted:
                raise AssertionError('stale explicit execution altered admitted work or human bytes')
            ack = observation(uart, f"schedule-admission {admitted['id']}", 'queued', 0)
            final = settled(uart, admitted)
            if final['state'] != 'cancelled':
                raise AssertionError('stale scheduling did not persist prevention')
            observed = check(data, final, b'first', node['id'])
            if (observed['nodes'][node['id']]['content'], observed['nodes'][node['id']]['version']) != (b'human-edit', edited['version']):
                raise AssertionError('scheduled stale candidate replaced the human edit')
            if len(observed['records']) != 1 or observed['files'][(4, 'other')] != b'untouched':
                raise AssertionError('stale prevention changed unrelated retained facts')
            uart.command('cat hello', 'human-edit')
            if counters(uart) != baseline:
                raise AssertionError('stale scheduling leaked resources')
        before = snapshot(data)[0]
        with session(mount, data, name + '-reboot') as uart:
            if status(uart.command(f"admission {admitted['id']}")) != final:
                raise AssertionError('reboot changed stale prevention')
            uart.command('cat hello', 'human-edit'); uart.command('cat other', 'untouched')
            uart.command(f"admission-activity {admitted['id']}", 'error: Unavailable')
        if snapshot(data)[0] != before:
            raise AssertionError('reboot replayed or rewrote stale work')
    return dict(case=name, verified=True, reboot_verified=True, no_replay=True,
                admitted=admitted, durable=[final], observations=[ack],
                conflict='Version', conflict_unchanged=True, reclaimed=True,
                previous_version=f"v_{node['version']:016x}", file_version=f"v_{edited['version']:016x}",
                file_sha256=hashlib.sha256(b'human-edit').hexdigest(), file_size=10,
                sha256=hashlib.sha256(before).hexdigest())
