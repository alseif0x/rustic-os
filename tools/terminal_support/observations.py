# SPDX-License-Identifier: Apache-2.0
"""Strict rendering and manual/deterministic parity for one native observation."""
from .activity_cases import act


def observe(uart, identity, kind, state, pending=0, requested=0):
    text = uart.command(f"observe-admission {identity['id']}")
    lines = text.replace('\r\n', '\n').splitlines()
    headers = [line for line in lines if line.startswith('admission-observation-v1 ')]
    if len(headers) != 1 or any(line.startswith('error:') for line in lines):
        raise AssertionError('missing or ambiguous coherent observation')
    prefix = (f"admission-observation-v1 profile=1 id={identity['id']} "
              f"service_instance={identity['instance']} kind={kind} ")
    suffix = (f"state={state} terminal={identity['terminal']}" if kind == 'retained' else
              f"phase={state} cancel_requested={requested} io_pending={pending}")
    if headers[0] != prefix + suffix:
        raise AssertionError(f'wrong coherent observation: {headers[0]}')
    completion = ([f"completion=op_{identity['lineage']}_{identity['terminal']:016x}"]
                  if kind == 'retained' and state == 'committed' else [])
    if [line for line in lines if line.startswith('completion=')] != completion:
        raise AssertionError('observation fabricated or lost its completion identity')
    value = dict(profile=1, id=identity['id'], instance=identity['instance'], kind=kind)
    if kind == 'retained':
        value.update(state=state, terminal=identity['terminal'])
    else:
        value.update(phase=state, pending=pending, requested=requested)
    return value


def paired(uart, pid, view):
    result = act(uart, pid, 'observe', view['id'])
    if view['kind'] == 'retained':
        expected = (dict(admitted=1, cancelled=2, committed=3)[view['state']], view['terminal'], 0)
    else:
        expected = (0x10 | dict(running=1, stopping=2, settling=3, queued=4)[view['phase']],
                    view['requested'], view['pending'])
    if (result['value'], result['other'], result['control_denied']) != expected or result['version'] != 0:
        raise AssertionError('manual and deterministic clients disagree on the same native observation')
    return result
