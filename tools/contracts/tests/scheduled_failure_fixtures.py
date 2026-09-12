# SPDX-License-Identifier: Apache-2.0
"""Synthetic negative-test inputs; never evidence of native execution."""
import hashlib


def failure_cases():
    result = []
    for name in ('late_header', 'late_flush', 'lost_stop', 'lost_result', 'restart_data',
                 'restart_flush', 'io_data', 'io_flush', 'pressure'):
        late = name in ('late_header', 'late_flush', 'restart_flush', 'io_flush')
        unknown = name in ('restart_data', 'io_data')
        states = ('admitted', 'admitted') if unknown else ('committed', 'cancelled')
        if name == 'lost_stop': states = ('cancelled', 'committed')
        if name in ('restart_flush', 'io_flush'): states = ('committed', 'admitted')
        if name == 'pressure': states = ('cancelled', 'cancelled')
        lineage = '08' * 16
        instance = f'si_{lineage}_0000000000000003'
        durable = [dict(id=f'ad_{lineage}_{i+3:016x}', lineage=lineage, number=i+3,
                        instance=instance, state=state, terminal=0 if state == 'admitted' else i+5)
                   for i, state in enumerate(states)]
        def observation(i, phase, pending, requested=0):
            return dict(id=durable[i]['id'], instance=instance, phase=phase, pending=pending, requested=requested)
        phase = 'settling' if late else 'running'
        obs = [observation(0, 'queued', 0), observation(0, phase, 1), observation(1, 'queued', 0)]
        if name == 'pressure': obs.append(observation(1, 'queued', 0, 1))
        if name.startswith(('late_', 'io_')) or name in ('lost_stop', 'pressure'):
            obs.append(observation(0, 'settling' if late else 'stopping', 1, 1))
        completions = []
        for i, status in enumerate(durable):
            if status['state'] != 'committed': continue
            payload = (b'first', b'second')[i]
            completions.append(dict(operation_id=f"op_{lineage}_{status['terminal']:016x}",
                service_instance=instance, state='succeeded', effect='committed', cancel_requested=False,
                receipt=dict(workspace='workspace_a', resource='file_a', previous_version='v_0000000000000002',
                             version=f"v_{status['terminal']:016x}", size=len(payload), sha256=hashlib.sha256(payload).hexdigest(),
                             retry=dict(epoch='epoch_a', key='key_a'))))
        content = b'second' if name == 'lost_stop' else b'first' if completions else b'before'
        case = dict(case='scheduled_failure_' + name, verified=True, reboot_verified=True, no_replay=True,
                    durable=durable, observations=obs, completions=completions, sha256='b'*64,
                    previous_version='v_0000000000000002', file_sha256=hashlib.sha256(content).hexdigest(),
                    file_size=len(content), file_version=completions[-1]['receipt']['version'] if completions else 'v_0000000000000002',
                    skip=15 if name == 'late_header' else 16 if late else 0,
                    old_service_pid=3, new_service_pid=7, query_error='Uncertain',
                    terminal_reply_ready=name == 'lost_result')
        case.update({flag: True for flag in ('reclaimed', 'cancel_only', 'discarded_reply', 'stale_reply_rejected',
            'restart_drained_io', 'owner_progress', 'pending_abandoned', 'reconciled_after_restart',
            'staging_full', 'retained_full', 'undrained_client', 'staging_retained')})
        result.append(case)
    return result
