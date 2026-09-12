# SPDX-License-Identifier: Apache-2.0
"""Check native profile selection against the exact reviewed v2 descriptor bundle."""
from .lifecycle_conformance import catalog, native_check as lifecycle_native, exchange
from .validation import require, ContractError


def native_check(terminal):
    from tools.terminal_support.negotiation_report import verify
    from tools.terminal_support.selection_report import verify as verify_selection
    c = catalog()
    # Preserve the independent lifecycle/disk prerequisites, not just a success flag.
    lifecycle_native(c, terminal)
    report = terminal.get('negotiation')
    try:
        verify(report)
        verify_selection(report['selection'])
    except (AssertionError, KeyError, TypeError, ValueError) as error:
        raise ContractError('invalid negotiated lifecycle evidence') from error
    for key, group in [('before_upgrade','profiles'), ('legacy','profiles'), ('restart','before'), ('restart','after'), ('reboot','profiles')]:
        for descriptor in report[key][group]:
            require(descriptor['sha256'] == c.digest(descriptor['method']), 'native descriptor differs from reviewed bundle')
    for result in (report['legacy']['prepared']['operation'], report['legacy']['terminal']['operation'],
                   report['restart']['operation']['operation'], report['restart']['after_operation']['operation'], report['reboot']['operation']['operation']):
        exchange(c, 'operations.get', result)
    for name in ('accepted', 'too_late'):
        item = report['legacy'][name]
        exchange(c, 'operations.cancel', {k:v for k,v in item.items() if k != 'client'})
    for session in report['selection']:
        for name in ('running', 'terminal'):
            exchange(c, 'operations.get', session[name]['operation'])
        if session['case'] == 'cancel':
            exchange(c, 'operations.cancel', {k:v for k,v in session['ack'].items() if k != 'client'})
    return dict(status='success', version=2, profile=1, backend='native_negotiated_lifecycle',
                descriptors=10, inspections=5, cancellations=2, guest_execution=True,
                selected_sessions=3, selected_inspections=6, selected_cancellations=1,
                build_id=terminal['build_id'], kernel_sha256=terminal['kernel_sha256'])
