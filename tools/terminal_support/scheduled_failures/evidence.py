# SPDX-License-Identifier: Apache-2.0
"""Retained facts, receipt lookup and independent bytes for scheduled failure cuts."""
import hashlib
import time
from ..admission_cases import status, check
from ..activity_cases import held
from ..oracle import snapshot
from ..scheduling_cases import observation, receipt


def queue_pair(uart, admissions, skip):
    first, second = admissions
    uart.command(f"hold-io {skip} 400", "diagnostic armed")
    ack = observation(uart, f"schedule-admission {first['id']}", "queued", 0)
    held(uart)
    active = observation(uart, f"admission-activity {first['id']}", "settling" if skip >= 15 else "running", 1)
    pending = observation(uart, f"schedule-admission {second['id']}", "queued", 0)
    return [ack, active, pending]


def await_uncertain(uart, admission):
    deadline = time.monotonic() + 12
    while True:
        output = uart.command(f"admission {admission['id']}", "")
        errors = [line for line in output.replace('\r\n', '\n').splitlines() if line.startswith('error:')]
        if errors == ['error: Uncertain'] and 'admission-v1 ' not in output:
            return 'Uncertain'
        if errors != ['error: Busy'] or 'admission-v1 ' in output or time.monotonic() >= deadline:
            raise AssertionError(f"submitted fault did not become uncertain: {output}")
        time.sleep(.02)


def retained(uart, expected):
    observed = [status(uart.command(f"admission {a['id']}")) for a in expected]
    if observed != expected:
        raise AssertionError("query/recovery changed retained scheduling facts")
    return observed


def completions(uart, finals):
    return [receipt(uart, final) for final in finals if final['state'] == 'committed']


def proof(data, node, finals, content):
    for final, payload in zip(finals, (b'first', b'second')):
        check(data, final, payload, node['id'])
    _, disk = snapshot(data)
    current = disk['nodes'][node['id']]
    commits = [s for s in finals if s['state'] == 'committed']
    expected_version = commits[-1]['terminal'] if commits else node['version']
    if (current['content'], current['version']) != (content, expected_version):
        raise AssertionError("file effect/version contradicts independent scheduled disk evidence")
    if disk['files'][(4, 'other')] != b'untouched' or len(disk['records']) != 2:
        raise AssertionError("scheduled failure changed unrelated files or retained inventory")
    return dict(sha256=disk['selected_sha256'], file_sha256=hashlib.sha256(content).hexdigest(),
                file_version=f"v_{current['version']:016x}", previous_version=f"v_{node['version']:016x}",
                file_size=len(content))


def reboot(session, mount, data, name, finals, content):
    before = snapshot(data)[0]
    with session(mount, data, name + '-reboot') as uart:
        retained(uart, finals)
        uart.command('cat hello', content.decode())
        uart.command('cat other', 'untouched')
        for final in finals:
            uart.command(f"admission-activity {final['id']}", 'error: Unavailable')
    if snapshot(data)[0] != before:
        raise AssertionError("reboot or read-only reconciliation rewrote the volume")
