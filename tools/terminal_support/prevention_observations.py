# SPDX-License-Identifier: Apache-2.0
"""Actual profile-2 IPC, old-client compatibility and independent native clients."""
from .observations import observe, paired
from .activity_cases import act, held
from .authority_cases import cleanup, fence
from .cases import counters, pid
from .oracle import snapshot
from .scheduling_cases import settled
from . import lifecycle_observations as logical


def retained(uart, result, reason):
    legacy = observe(uart, result, 'retained', result['state'])
    view = observe(uart, result, 'retained', result['state'], profile=2, prevention=reason)
    if {k: v for k, v in view.items() if k not in ('prevention', 'profile')} != {
            k: v for k, v in legacy.items() if k != 'profile'}:
        raise AssertionError('observation profiles disagree')
    return view


def paired_retained(uart, results, reasons):
    baseline = counters(uart)
    inspector = pid(uart, 'admission-session hello /config/owner-policy 4')
    try:
        views = [retained(uart, r, c) for r, c in zip(results, reasons, strict=True)]
        clients = [paired(uart, inspector, v) for v in views]
        lifecycle = [logical.retained(uart, v, inspector) for v in views]
    finally:
        cleanup(uart, inspector)
    if counters(uart) != baseline:
        raise AssertionError('observation client leaked resources')
    return dict(views=views, clients=clients, lifecycle=lifecycle)


def authority(uart, data, admitted):
    baseline = counters(uart)
    executor = pid(uart, 'admission-session hello /config/owner-policy 7')
    canceller = pid(uart, 'admission-session hello /config/owner-policy 8')
    uart.command('hold-io 0 400', 'diagnostic armed')
    act(uart, executor, 'schedule', admitted['id'])
    held(uart)
    live = observe(uart, admitted, 'active', 'running', pending=1, profile=2)
    client = paired(uart, executor, live)
    denied = act(uart, canceller, 'observe-v2', admitted['id'], 17)
    if any(denied[k] for k in ('value', 'other', 'control_denied', 'version')):
        raise AssertionError('CANCEL-only observation disclosed data')
    fence(uart, executor, 'access=fenced members=1')
    final = settled(uart, admitted)
    before = snapshot(data)[0]
    view = retained(uart, final, 'authority_lost')
    revoked = act(uart, executor, 'observe-v2', admitted['id'], 18)
    if any(revoked[k] for k in ('value', 'other', 'control_denied', 'version')):
        raise AssertionError('revoked observation disclosed data')
    if snapshot(data)[0] != before:
        raise AssertionError('observation changed the retained authority cause')
    cleanup(uart, executor, canceller)
    if counters(uart) != baseline:
        raise AssertionError('authority observation leaked resources')
    pair = paired_retained(uart, [final], ['authority_lost'])
    return final, dict(live=live, live_client=client, retained=view, paired=pair,
                       cancel_only=denied, revoked=revoked)
