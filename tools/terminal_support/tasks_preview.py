# SPDX-License-Identifier: Apache-2.0
"""Actual native edit candidates must match expected rows without any disk effect."""
from .cases import counters, pid
from .authority_cases import cleanup
from .oracle import snapshot
from .tasks_cases import write_document, observe

DOCUMENT = "rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n"
ROWS = ("7 [open] Review kernel", "42 [done] Boot the OS")


def candidate(uart, data, command, task_id, changed, rows, document=DOCUMENT):
    _, state = snapshot(data)
    source = [node for node in state['nodes'].values() if node['content'] == document.encode('ascii')]
    assert len(source) == 1
    expected = f"preview task={task_id} changed={int(changed)} source_version={source[0]['version']}; not applied"
    result = observe(uart, data, command, expected, rows)
    result.update(task_id=task_id, changed=changed, source_version=source[0]['version'])
    return result


def exercise(uart, data):
    baseline = counters(uart)
    _, initial = snapshot(data)
    write_document(uart, DOCUMENT)
    cases = []
    for _ in range(2):
        cases.append(candidate(uart, data, 'tasks preview add tasks-fixture "Ship Rust"', 43, True,
                               (*ROWS, '43 [open] Ship Rust')))
    cases.append(candidate(uart, data, 'tasks preview done tasks-fixture 7', 7, True,
                           ('7 [done] Review kernel', ROWS[1])))
    cases.append(candidate(uart, data, 'tasks preview done tasks-fixture 42', 42, False, ROWS))
    observe(uart, data, 'tasks list tasks-fixture', '2 tasks', ROWS)
    for command, error in (
        ('tasks preview done tasks-fixture 999', 'NotFound'),
        ('tasks preview done tasks-fixture 0', 'invalid arguments; type help'),
        ('tasks preview done tasks-fixture 07', 'invalid arguments; type help'),
        ('tasks preview add tasks-fixture ""', 'invalid arguments; type help'),
        ('tasks preview add tasks-fixture "bad\\ttitle"', 'invalid arguments; type help'),
        ('tasks preview add tasks-fixture "1234567890123456789012345"', 'invalid arguments; type help'),
        ('tasks preview add missing "Missing"', 'NotFound'),
    ):
        cases.append(observe(uart, data, command, 'error: ' + error))
    first, second = pid(uart, 'run spin'), pid(uart, 'run spin')
    cases.append(observe(uart, data, 'tasks preview done tasks-fixture 7', 'error: service busy or full'))
    cleanup(uart, first, second)
    write_document(uart, 'rustic-tasks-v1\n')
    cases.append(candidate(uart, data, 'tasks preview add tasks-fixture "First"', 1, True,
                           ('1 [open] First',), 'rustic-tasks-v1\n'))
    write_document(uart, 'rustic-tasks-v1\n4294967295\topen\tLast ID\n')
    cases.append(observe(uart, data, 'tasks preview add tasks-fixture "Overflow"', 'error: Exhausted'))
    maximum = 'rustic-tasks-v1\n' + ''.join(f'{i}\topen\tTask {i}\n' for i in range(1, 17))
    write_document(uart, maximum)
    cases.append(observe(uart, data, 'tasks preview add tasks-fixture "Full"', 'error: tasks capacity exceeded'))
    write_document(uart, 'rustic-tasks-v1\n7\topen\tValid\nmalformed tail\n')
    cases.append(observe(uart, data, 'tasks preview done tasks-fixture 7', 'error: invalid tasks document'))
    write_document(uart, DOCUMENT)
    uart.command('write /config/owner-policy invalid')
    uart.command('restart files', 'utility sessions revoked')
    cases.append(observe(uart, data, 'tasks preview done tasks-fixture 7', 'error: service denied'))
    uart.command(r'write /config/owner-policy "rustic-owner-v1\nhelpers=explicit\n"')
    uart.command('restart files', 'utility sessions revoked')
    uart.command('rm tasks-fixture')
    _, final = snapshot(data)
    assert final['files'] == initial['files'] and counters(uart) == baseline
    return {'verified': True, 'read_only': True, 'cases': cases}


def after_reboot(uart, data):
    write_document(uart, DOCUMENT)
    result = candidate(uart, data, 'tasks preview done tasks-fixture 7', 7, True,
                       ('7 [done] Review kernel', ROWS[1]))
    uart.command('rm tasks-fixture')
    return result
