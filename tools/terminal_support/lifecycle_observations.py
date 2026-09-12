# SPDX-License-Identifier: Apache-2.0
"""Read actual SDK output and compare the independently bound diagnostic client."""
from .activity_cases import act

STATES = dict(prepared=1, queued=2, running=3, reconciling=4,
              succeeded=5, cancelled=6, failed=7, prevented=8)


def inspect(uart, identity, state, inspector=None, stop=None, failure=None, completion=None, negotiated=False, selected=False):
    command = 'inspect-selected' if selected else 'inspect-negotiated' if negotiated else 'inspect-operation'
    text = uart.command(f"{command} {identity['id']}")
    lines = [line.strip() for line in text.splitlines() if line.startswith('operation-v2 ')]
    if len(lines) != 1:
        raise AssertionError('missing unique typed lifecycle reply')
    parts = lines[0].split()[1:]
    value = dict(part.split('=', 1) for part in parts)
    if len(value) != len(parts):
        raise AssertionError('duplicate logical result field')
    expected = dict(id=identity['id'], service_instance=identity['instance'], state=state,
                    effect='committed' if state == 'succeeded' else 'unknown' if state == 'reconciling' else 'none')
    if stop is not None:
        expected['stop_pending'] = str(int(stop))
    if failure is not None:
        expected['failure'] = failure
    if completion is not None:
        expected['completion_id'] = completion
    if value != expected:
        raise AssertionError(f'logical lifecycle mismatch: {value!r} != {expected!r}')
    value['operation_id'] = value.pop('id')
    if stop is not None:
        value['stop_pending'] = bool(stop)
    result = dict(operation=value)
    if inspector is not None:
        client = act(uart, inspector, 'inspect-selected' if selected else 'inspect-negotiated' if negotiated else 'inspect', identity['id'])
        detail = int(completion.rsplit('_', 1)[1], 16) if completion else int(bool(stop))
        if client != dict(status=0, value=STATES[state], other=detail, control_denied=0,
                          version={None: 0, 'version_conflict': 1, 'access_denied': 2}[failure]):
            raise AssertionError('manual and deterministic logical clients disagree')
        result['client'] = client
    return result


def retained(uart, view, inspector):
    state, failure, completion = 'prepared', None, None
    if view['state'] == 'committed':
        state = 'succeeded'
        completion = 'op_' + view['id'][3:36] + f"{view['terminal']:016x}"
    elif view['state'] == 'cancelled':
        state = dict(unknown='prevented', requested='cancelled', version_conflict='failed', authority_lost='failed')[view['prevention']]
        failure = dict(version_conflict='version_conflict', authority_lost='access_denied').get(view['prevention'])
    return inspect(uart, view, state, inspector, failure=failure, completion=completion)


def cancel(uart, identity, disposition, actor=None, negotiated=False, selected=False):
    if actor is None:
        command = 'cancel-selected' if selected else 'cancel-negotiated' if negotiated else 'request-operation-cancel'
        text = uart.command(f"{command} {identity['id']}")
        lines = [line.strip() for line in text.splitlines() if line.startswith('operation-cancel-v2 ')]
        if lines != [f"operation-cancel-v2 id={identity['id']} disposition={disposition}"]:
            raise AssertionError('cancel ACK disclosed extra fields or wrong disposition')
        return dict(operation_id=identity['id'], disposition=disposition)
    client = act(uart, actor, 'cancel-selected' if selected else 'cancel-negotiated' if negotiated else 'cancel', identity['id'])
    if client != dict(status=0, value=dict(requested=1, already_requested=2, too_late=3)[disposition],
                      other=0, control_denied=0, version=0):
        raise AssertionError('CANCEL-only ACK disagreed or disclosed inspection fields')
    return dict(operation_id=identity['id'], disposition=disposition, client=client)


def denied(uart, actor, action, identity, code=17):
    result = act(uart, actor, action, identity['id'], code)
    if result != dict(status=code, value=0, other=0, control_denied=0, version=0):
        raise AssertionError('denial disclosed operation details')
    return result
