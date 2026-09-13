# SPDX-License-Identifier: Apache-2.0
"""Native owner-client writes with separate receipt inventory and disk oracle."""
import json
import re
import tempfile
from pathlib import Path
from boot_support.image import package
from .machine import machine, disk
from .connection import Connection
from .failure import preserve_failure
from .oracle import snapshot
from .cases import counters
from .tasks_cases import write_document
from .read_cases import references

DOCUMENT = 'rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n'
ADDED = DOCUMENT + '43\topen\tShip Rust\n'
DONE = ADDED.replace('7\topen\t', '7\tdone\t')


def state(data, expected, *, pending=False):
    _, value = snapshot(data)
    assert value['files'][(4, 'tasks-fixture')] == expected.encode('ascii')
    journal = value['files'].get((3, 'tasks-intent'), b'')
    assert bool(journal) == pending, ('intent presence', pending)
    return value


def key(output):
    match = re.search(r'task intent=(\d+) retained before submission', output)
    assert match, output
    return int(match.group(1))


def retained_key(value):
    journal = value['files'][(3, 'tasks-intent')]
    assert journal.startswith(b'RTSKI001') and len(journal) > 136
    return next(node['version'] for node in value['nodes'].values() if node['content'] == journal)


def receipt(value, intent, expected):
    records = [record for record in value['records'] if record['key'] == intent]
    assert len(records) == 1
    record = records[0]
    assert record['subject'] == 1 and record['workspace'] == 4
    assert record['committed'] > intent and record['content'] == expected.encode('ascii')
    return record['committed']


def failure(uart, command, expected):
    output = uart.command(command, 'error: ' + expected)
    assert [line for line in output.splitlines() if line.startswith('error:')] == ['error: ' + expected]
    uart.command('status', '1')
    return output


def first(uart, data):
    baseline = counters(uart)
    write_document(uart, DOCUMENT)
    initial, _ = snapshot(data)
    failure(uart, 'tasks add tasks-fixture "Ship Rust"', 'task writes require explicit setup: tasks enable')
    assert snapshot(data)[0] == initial
    uart.command('tasks enable', 'persistent storage format is at least v3')
    first_key = key(uart.command('tasks add tasks-fixture "Ship Rust"', 'task=43 applied'))
    first_state = state(data, ADDED)
    receipt(first_state, first_key, ADDED)
    uart.command('tasks list tasks-fixture', '43 [open] Ship Rust')
    second_key = key(uart.command('tasks done tasks-fixture 7', 'task=7 applied'))
    assert second_key > first_key
    state(data, DONE)
    before, _ = snapshot(data)
    uart.command('tasks done tasks-fixture 7', 'task=7 unchanged')
    assert snapshot(data)[0] == before, 'no-op consumed storage or receipt'
    failure(uart, 'tasks add tasks-fixture "Full"', 'Full')
    state(data, DONE)
    uart.command('tasks recover', 'No retained task intent')
    # Deliberate owner retention rotation; never performed by task commands.
    uart.command('rotate-receipts')
    write_document(uart, DOCUMENT)
    failure(uart, 'tasks-write-acceptance conflict done tasks-fixture 7', 'Version')
    state(data, 'human edit survives')
    write_document(uart, DOCUMENT)
    pending_key = key(failure(uart, 'tasks-write-acceptance prepared add tasks-fixture "Never submitted"', 'Uncertain'))
    state(data, DOCUMENT, pending=True)
    before, _ = snapshot(data)
    failure(uart, 'tasks recover', 'OutcomeUnknown')
    failure(uart, 'tasks add tasks-fixture "Blocked"', f'task intent={pending_key} remains unresolved; use tasks recover')
    assert snapshot(data)[0] == before, 'recovery replayed or rebased an absent outcome'
    failure(uart, f'tasks forget {pending_key + 1}', f'task intent={pending_key} remains unresolved; use tasks recover')
    uart.command(f'tasks forget {pending_key}', 'does not cancel or undo')
    state(data, DOCUMENT)
    failure(uart, 'tasks-write-acceptance lost-journal add tasks-fixture "Never submitted"', 'Uncertain')
    journal_state = state(data, DOCUMENT, pending=True)
    journal_key = retained_key(journal_state)
    assert not journal_state['records'], 'lost journal reply reached target submission'
    uart.command('restart files', 'utility sessions revoked')
    failure(uart, 'tasks recover', 'OutcomeUnknown')
    state(data, DOCUMENT, pending=True)
    uart.command(f'tasks forget {journal_key}', 'does not cancel or undo')
    lost_key = key(failure(uart, 'tasks-write-acceptance lost-reply add tasks-fixture "Ship Rust"', 'Uncertain'))
    assert lost_key > pending_key
    receipt(state(data, ADDED, pending=True), lost_key, ADDED)
    failure(uart, 'tasks recover', 'Unavailable')
    uart.command('restart files', 'utility sessions revoked')
    uart.command('tasks recover', 'task=43 recovered')
    state(data, ADDED)
    assert counters(uart) == baseline
    # A second lost reply remains retained across an actual VM reboot.
    reboot_key = key(failure(uart, 'tasks-write-acceptance lost-reply done tasks-fixture 7', 'Uncertain'))
    state(data, DONE, pending=True)
    return {'first_key': first_key, 'second_key': second_key, 'prepared_key': pending_key,
            'lost_journal_key': journal_key, 'lost_key': lost_key, 'reboot_key': reboot_key,
            'first_commit_sequence': first_state['sequence'], 'baseline': baseline}


def second(uart, data, result):
    before = state(data, DONE, pending=True)
    uart.command('tasks recover', 'task=7 recovered')
    after = state(data, DONE)
    # Only the intent cleanup commits. Recovery never repeats the target effect.
    target_before = next(node for node in before['nodes'].values() if node['content'] == DONE.encode())
    target_after = next(node for node in after['nodes'].values() if node['content'] == DONE.encode())
    assert target_before == target_after
    uart.command('tasks list tasks-fixture', '7 [done] Review kernel')
    uart.command('tasks recover', 'No retained task intent')
    assert counters(uart) == result['baseline']
    result['reboot_target_version'] = target_after['version']
    uart.command('rotate-receipts')
    # Full valid document spans every candidate relay chunk, including the tail.
    maximum = 'rustic-tasks-v1\n' + ''.join(f'{4000000000+i}\topen\t{"x" * 24}\n' for i in range(16))
    write_document(uart, maximum)
    maximum_done = maximum.replace('4000000000\topen\t', '4000000000\tdone\t')
    key(uart.command('tasks done tasks-fixture 4000000000', 'task=4000000000 applied'))
    state(data, maximum_done)
    failure(uart, 'tasks add tasks-fixture "Overflow"', 'tasks capacity exceeded')
    uart.command('rotate-receipts')
    write_document(uart, DOCUMENT)
    # Create an actual older operation using the future journal's numeric key.
    uart.command('write other "before collision"')
    refs = references(uart, '/workspaces', 'other')
    _, current = snapshot(data)
    other = next(node for node in current['nodes'].values() if node['content'] == b'before collision')
    collision_key = current['sequence'] + 2
    uart.command(f'replace-ref {refs["workspace"]} {refs["resource"]} v_{other["version"]:016x} e_{current["epoch"]:016x} k_{collision_key:016x} "old operation"')
    failure(uart, 'tasks add tasks-fixture "Ship Rust"', 'IdempotencyConflict')
    collision = state(data, DOCUMENT, pending=True)
    assert retained_key(collision) == collision_key
    before, _ = snapshot(data)
    failure(uart, 'tasks recover', 'Uncertain')
    assert snapshot(data)[0] == before
    uart.command(f'tasks forget {collision_key}', 'does not cancel or undo')
    uart.command('rotate-receipts')
    human_key = key(failure(uart, 'tasks-write-acceptance lost-reply add tasks-fixture "Ship Rust"', 'Uncertain'))
    uart.command('restart files', 'utility sessions revoked')
    uart.command('write tasks-fixture "later human edit"')
    before, _ = snapshot(data)
    failure(uart, 'tasks recover', 'Version')
    assert snapshot(data)[0] == before
    state(data, 'later human edit', pending=True)
    uart.command('rotate-receipts')
    before, _ = snapshot(data)
    failure(uart, 'tasks recover', 'ExpiredEpoch')
    assert snapshot(data)[0] == before
    uart.command(f'tasks forget {human_key}', 'does not cancel or undo')
    write_document(uart, DOCUMENT)
    uart.command('write /config/owner-policy invalid')
    uart.command('restart files', 'utility sessions revoked')
    before, _ = snapshot(data)
    failure(uart, 'tasks add tasks-fixture "Denied"', 'service denied')
    assert snapshot(data)[0] == before
    uart.command(r'write /config/owner-policy "rustic-owner-v1\nhelpers=explicit\n"')
    uart.command('restart files', 'utility sessions revoked')
    assert counters(uart) == result['baseline']
    result.update(maximum_candidate_bytes=len(maximum_done), collision_key=collision_key,
                  human_edit_key=human_key, expired_epoch_refused=True)
    result['verified'] = True


def verify(image, output):
    image, output = Path(image).resolve(), Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / 'image.json').read_text())
    if metadata.get('tasks_acceptance') is not True:
        raise ValueError('task write cuts require the explicit acceptance build')
    mount = package(image.parent / 'kernel.elf', 'terminal', metadata['build_id'], {})
    with tempfile.TemporaryDirectory(prefix='rustic-tasks-write-') as directory:
        directory = Path(directory)
        with disk(directory / 'data.raw', True) as data, preserve_failure(data, output, 'tasks-write', metadata):
            for phase, boot in enumerate((image, mount), 1):
                sock = directory / f'uart-{phase}.sock'
                with machine(boot, data, f'unix:{sock},server=on,wait=off', output / f'qemu-{phase}.log') as vm:
                    uart = Connection(sock, vm, output / f'serial-{phase}.log', 30)
                    try:
                        uart.until()
                        if phase == 1:
                            result = first(uart, data)
                        else:
                            second(uart, data, result)
                        uart.send(b'exit\r')
                        uart.until(b'RUSTIC TERMINAL stopped=1 reclaimed=1')
                        assert vm.wait(timeout=10) == 33
                    finally:
                        uart.close()
            selected, final = snapshot(data)
            (output / 'files.bin').write_bytes(selected)
    evidence = {'verified': True, 'boots': 2, 'build_id': metadata['build_id'],
                'kernel_sha256': metadata['kernel_sha256'], 'result': result,
                'final_sequence': final['sequence']}
    (output / 'tasks-write.json').write_text(json.dumps(evidence, separators=(',', ':')) + '\n')
    print(json.dumps(evidence))
    return evidence
