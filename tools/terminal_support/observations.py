# SPDX-License-Identifier: Apache-2.0
"""Strict rendering and manual/deterministic parity for one native observation."""
from .activity_cases import act


def observe(uart, identity, kind, state, pending=0, requested=0, profile=1, prevention=None):
    if profile not in (1, 2) or (prevention is not None and (profile != 2 or kind != 'retained')):
        raise AssertionError('invalid requested observation profile/variant')
    if profile == 2 and kind == 'retained' and (
            prevention not in ('none', 'unknown', 'requested', 'version_conflict', 'authority_lost') or
            (state == 'cancelled') != (prevention != 'none')):
        raise AssertionError('cause contradicts retained state')
    command = 'observe-admission' if profile == 1 else 'observe-admission-v2'
    text = uart.command(f"{command} {identity['id']}")
    lines = text.replace('\r\n', '\n').splitlines()
    headers = [line for line in lines if line.startswith('admission-observation-v')]
    if len(headers) != 1 or any(line.startswith('error:') for line in lines):
        raise AssertionError('missing or ambiguous coherent observation')
    prefix = (f"admission-observation-v{profile} profile={profile} id={identity['id']} "
              f"service_instance={identity['instance']} kind={kind} ")
    suffix = (f"state={state} terminal={identity['terminal']}" if kind == 'retained' else
              f"phase={state} cancel_requested={requested} io_pending={pending}")
    if prevention is not None:
        suffix += f' prevention={prevention}'
    if headers[0] != prefix + suffix:
        raise AssertionError(f'wrong coherent observation: {headers[0]}')
    completion = ([f"completion=op_{identity['lineage']}_{identity['terminal']:016x}"]
                  if kind == 'retained' and state == 'committed' else [])
    if [line for line in lines if line.startswith('completion=')] != completion:
        raise AssertionError('observation fabricated or lost its completion identity')
    value = dict(profile=profile, id=identity['id'], instance=identity['instance'], kind=kind)
    if kind == 'retained':
        value.update(state=state, terminal=identity['terminal'])
        if prevention is not None:
            value['prevention'] = prevention
    else:
        value.update(phase=state, pending=pending, requested=requested)
    return value


def paired(uart, pid, view):
    result = act(uart, pid, 'observe' if view['profile'] == 1 else 'observe-v2', view['id'])
    if view['kind'] == 'retained':
        expected = (dict(admitted=1, cancelled=2, committed=3)[view['state']], view['terminal'], 0)
    else:
        expected = (0x10 | dict(running=1, stopping=2, settling=3, queued=4)[view['phase']],
                    view['requested'], view['pending'])
    cause = dict(none=0, unknown=1, requested=2, version_conflict=3, authority_lost=4).get(view.get('prevention'), 0)
    if (result['value'], result['other'], result['control_denied']) != expected or result['version'] != cause:
        raise AssertionError('manual and deterministic clients disagree on the same native observation')
    return result
