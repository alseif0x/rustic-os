# SPDX-License-Identifier: Apache-2.0
"""Version-2 lifecycle validation, kept separate from historical v1 semantics."""
import json
import re
from jsonschema import ValidationError
from .catalog import Catalog, ROOT
from .validation import ContractError, parse_message, require


def catalog():
    return Catalog(ROOT.parent / 'v2')


def identity(token):
    _, lineage, number = token.split('_')
    require(int(lineage, 16) != 0 and int(number, 16) != 0, 'zero identity')
    return lineage, int(number, 16)


def validate_exchange(c, request, response):
    require(c.version == 2, 'wrong lifecycle contract version')
    # Apply the shared canonical message limits even to in-memory callers.
    request, response = (parse_message(json.dumps(v).encode()) for v in (request, response))
    method = request.get('method')
    require(method in c.entries, 'unknown lifecycle method')
    try:
        c.validator(method, 'request').validate(request)
        c.validator(method, 'response').validate(response)
    except ValidationError as error:
        raise ContractError('invalid lifecycle shape') from error
    requested = request['params']['operation_id']
    lineage, number = identity(requested)
    if 'error' in response:
        return
    result = response['result']
    require(result['operation_id'] == requested, 'operation identity changed')
    if method == 'operations.get':
        origin, sequence = identity(result['service_instance'])
        require(origin == lineage and sequence <= number, 'invalid originating service instance')
        if result['state'] == 'succeeded':
            origin, sequence = identity(result['completion_id'])
            require(origin == lineage and sequence > number, 'invalid completion reference')


def exchange(c, method, result):
    request = dict(version=2, method=method, params=dict(operation_id=result['operation_id']))
    response = dict(version=2, method=method, result=result)
    validate_exchange(c, request, response)


def host_check(c):
    lineage = '07' * 16
    common = dict(operation_id=f'ad_{lineage}_0000000000000009', service_instance=f'si_{lineage}_0000000000000008')
    count = 0
    for state in ('prepared', 'queued', 'running', 'reconciling', 'succeeded', 'cancelled', 'failed', 'prevented'):
        result = dict(common, state=state, effect='committed' if state == 'succeeded' else 'unknown' if state == 'reconciling' else 'none')
        if state in ('queued', 'running', 'reconciling'):
            result['stop_pending'] = True
        if state == 'succeeded':
            result['completion_id'] = f'op_{lineage}_000000000000000c'
        if state == 'failed':
            result['failure'] = 'access_denied'
        exchange(c, 'operations.get', result)
        count += 1
    for disposition in ('requested', 'already_requested', 'too_late'):
        exchange(c, 'operations.cancel', dict(operation_id=common['operation_id'], disposition=disposition))
        count += 1
    return dict(status='success', version=2, backend='host_lifecycle_contract', exchanges=count, guest_execution=False)


def native_check(c, terminal):
    from tools.terminal_support.lifecycle_report import verify
    require(terminal.get('verified') is True and terminal.get('boots') == 2, 'missing native terminal evidence')
    require(isinstance(terminal.get('kernel_sha256'), str) and re.fullmatch('[0-9a-f]{64}', terminal['kernel_sha256']), 'missing kernel identity')
    prevention = terminal.get('prevention')
    try:
        verify(prevention)
    except (AssertionError, KeyError, TypeError, ValueError) as error:
        raise ContractError('incomplete or contradictory native lifecycle evidence') from error
    inspected = cancelled = 0
    def walk(value):
        nonlocal inspected, cancelled
        if isinstance(value, list):
            for child in value:
                walk(child)
        elif isinstance(value, dict):
            if 'operation' in value:
                exchange(c, 'operations.get', value['operation'])
                inspected += 1
            elif 'disposition' in value:
                exchange(c, 'operations.cancel', {k: v for k, v in value.items() if k != 'client'})
                cancelled += 1
            else:
                for child in value.values():
                    walk(child)
    walk(prevention)
    return dict(status='success', version=2, backend='native_sdk_lifecycle_evidence',
                guest_execution=True, inspected=inspected, cancellation_acks=cancelled,
                kernel_sha256=terminal['kernel_sha256'], catalog_advertised=False,
                scope='retained_admission_inspection_and_minimal_stop_ack')
