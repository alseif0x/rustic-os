# SPDX-License-Identifier: Apache-2.0
"""Native selection and use; only the disposable terminal-test disk is inspected."""
from .authority_cases import actor, cleanup
from .cases import counters, pid
from .oracle import snapshot
from .scheduling_cases import settled
from .management_cases import start_restart, wait_job
from . import lifecycle_observations as logical

METHODS = ('operations.get', 'operations.cancel')


def profiles(uart, available, client=None):
    result = []
    for method in METHODS:
        text = uart.command(f'lifecycle-profile {method}')
        lines = [line for line in text.splitlines() if line.startswith('lifecycle-profile method=')]
        if len(lines) != 1:
            raise AssertionError('missing unique negotiated descriptor')
        pairs = lines[0].split()[1:]
        value = dict(part.split('=', 1) for part in pairs)
        if len(value) != len(pairs):
            raise AssertionError('duplicate descriptor field')
        for name in ('version', 'profile', 'responder', 'context', 'retained', 'tickets', 'active'):
            value[name] = int(value[name])
        if value['method'] != method or value['availability'] != ('available' if available else 'unavailable'):
            raise AssertionError('wrong method or mounted support')
        if client is not None:
            observed = actor(uart, client, 'profile-get' if method.endswith('.get') else 'profile-cancel')
            expected = dict(status=0, value=(1 if available else 3) | (2 << 8) | (2 << 16) | (1 << 24),
                            other=value['responder'], control_denied=1, version=2)
            if observed != expected:
                raise AssertionError('deterministic descriptor disagrees or digest was not checked')
            value['client'] = observed
        result.append(value)
    return result


def before_upgrade(uart, data):
    before = snapshot(data)[1]
    result = profiles(uart, False)
    if snapshot(data)[1]['selected_sha256'] != before['selected_sha256']:
        raise AssertionError('read-only discovery changed storage')
    return dict(profiles=result, disk_sha256=before['selected_sha256'])


def legacy_cancel(uart, data, admission):
    baseline = counters(uart)
    inspector = pid(uart, 'admission-session hello /config/owner-policy 4')
    canceller = pid(uart, 'admission-session hello /config/owner-policy 8')
    before = snapshot(data)[1]
    result = dict(profiles=profiles(uart, True, canceller),
                  prepared=logical.inspect(uart, admission, 'prepared', inspector, negotiated=True),
                  inspect_denied=logical.denied(uart, canceller, 'inspect-negotiated', admission),
                  cancel_denied=logical.denied(uart, inspector, 'cancel-negotiated', admission))
    if snapshot(data)[1]['selected_sha256'] != before['selected_sha256']:
        raise AssertionError('selection, inspection or refusal wrote disk')
    result['accepted'] = logical.cancel(uart, admission, 'requested', canceller, negotiated=True)
    terminal = settled(uart, admission)
    result['terminal'] = logical.inspect(uart, terminal, 'prevented', inspector, negotiated=True)
    result['too_late'] = logical.cancel(uart, terminal, 'too_late', negotiated=True)
    cleanup(uart, inspector, canceller)
    after = snapshot(data)[1]
    if after['format'] != 4 or after['files'] != before['files'] or after['nodes'] != before['nodes'] or counters(uart) != baseline:
        raise AssertionError('negotiated cancellation migrated storage, changed files or leaked resources')
    record = next(r for r in after['records'] if r.get('admission') == admission['number'])
    if record['state'] != 'cancelled' or 'prevention' in record or record['terminal'] != terminal['terminal']:
        raise AssertionError('v4 cancellation disagrees with independent retained evidence')
    result['format'] = after['format']
    result['disk'] = {k: record[k] for k in ('admission', 'state', 'terminal', 'committed')}
    return result


def restart(uart, data, admission):
    baseline = counters(uart)
    client = pid(uart, 'admission-session hello /config/owner-policy 4')
    before = snapshot(data)[1]
    result = dict(before=profiles(uart, True, client),
                  operation=logical.inspect(uart, admission, 'cancelled', client, negotiated=True))
    uart.command(f'revoke {client}', 'access=')
    result['revoked'] = actor(uart, client, 'profile-get', 18)
    job = start_restart(uart)
    wait_job(uart, job)
    # The supervisor retires old service clients rather than silently transferring grants.
    uart.command(f'actor-status {client}', 'denied')
    result['after'] = profiles(uart, True)
    result['retired_client'] = client
    result['after_operation'] = logical.inspect(uart, admission, 'cancelled', negotiated=True)
    if result['before'][0]['responder'] == result['after'][0]['responder']:
        raise AssertionError('service restart kept the old live responder')
    if result['operation']['operation'] != result['after_operation']['operation']:
        raise AssertionError('current responder replaced historical operation origin')
    after = snapshot(data)[1]
    if before['selected_sha256'] != after['selected_sha256'] or counters(uart) != baseline:
        raise AssertionError('discovery/restart wrote disk or retained a stale client')
    result['disk_sha256'] = after['selected_sha256']
    return result


def after_reboot(uart, data, report, admission):
    before = snapshot(data)[1]['selected_sha256']
    report['reboot'] = dict(profiles=profiles(uart, True),
                           operation=logical.inspect(uart, admission, 'cancelled', negotiated=True))
    if snapshot(data)[1]['selected_sha256'] != before:
        raise AssertionError('reboot discovery wrote disk')
    report['reboot']['disk_sha256'] = before
