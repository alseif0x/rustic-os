# SPDX-License-Identifier: Apache-2.0
"""Evaluate explicitly selected, hash-pinned historical reports without running jobs."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import math
from pathlib import Path
import re
import subprocess
import sys

from tools.research.failure_triage.format import (
    MAX_REQUEST_BYTES, RequestError, read_bounded_bytes, read_json_file, serialize_json,
)
from tools.research.failure_triage.prepare import build_request
from tools.research.failure_triage.schema import CLASSIFICATION_OPTIONS
from .__main__ import command, price_check, write, RESERVE_USD, MAX_USD

MAX_CASES = 32


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def load_catalog(path):
    catalog, raw, _ = read_json_file(path, limit=MAX_REQUEST_BYTES, label='catalog')
    if not isinstance(catalog, dict) or type(catalog.get('schema_version')) is not int or catalog['schema_version'] != 1:
        raise RequestError('invalid catalog version')
    cases = catalog.get('cases')
    if not isinstance(cases, list) or not 1 <= len(cases) <= MAX_CASES:
        raise RequestError('invalid case count')
    ids, groups, reports = set(), set(), set()
    for case in cases:
        if not isinstance(case, dict):
            raise RequestError('invalid case')
        ident, group = case.get('id'), case.get('incident_group')
        if not isinstance(ident, str) or not re.fullmatch(r'[a-z0-9][a-z0-9_-]{0,79}', ident) or ident in ids:
            raise RequestError('invalid or duplicate case ID')
        if not isinstance(group, str) or not group or group in groups:
            raise RequestError('duplicate or missing observation group')
        ids.add(ident)
        groups.add(group)
        if case.get('cohort') not in ('failure', 'control') or case.get('expected') not in CLASSIFICATION_OPTIONS:
            raise RequestError('invalid cohort or label')
        if case.get('report_kind') not in ('boot', 'sandbox', 'github-job'):
            raise RequestError('invalid report kind')
        if not isinstance(case.get('run_id'), str) or not case['run_id']:
            raise RequestError('missing run ID')
        if not isinstance(case.get('label_basis'), str) or not case['label_basis']:
            raise RequestError('missing label provenance')
        report = case.get('report')
        if not isinstance(report, dict) or not isinstance(report.get('path'), str):
            raise RequestError('invalid report reference')
        report_raw = read_bounded_bytes(Path(report['path']), limit=MAX_REQUEST_BYTES, label='native report')
        if digest(report_raw) != report.get('sha256'):
            raise RequestError('native report hash mismatch')
        report_digest = digest(report_raw)
        if report_digest in reports:
            raise RequestError('duplicate native report')
        reports.add(report_digest)
        logs = case.get('logs')
        if not isinstance(logs, list) or not 1 <= len(logs) <= 32:
            raise RequestError('invalid log selection')
        for log in logs:
            if not isinstance(log, dict) or not isinstance(log.get('path'), str):
                raise RequestError('invalid log reference')
            start, end = log.get('start'), log.get('end')
            if type(start) is not int or type(end) is not int or not 1 <= start <= end:
                raise RequestError('invalid log range')
            lines = read_bounded_bytes(Path(log['path']), label='log').decode('utf-8').splitlines(keepends=True)
            if end > len(lines) or digest(''.join(lines[start-1:end]).encode()) != log.get('sha256'):
                raise RequestError('log range/hash mismatch')
    return catalog, raw


def metrics(rows, key):
    accepted = [r for r in rows if r[key] and r[key]['accepted']]
    correct = sum(r[key]['category'] == r['expected'] for r in accepted)
    return {'cases': len(rows), 'accepted': len(accepted), 'correct_accepted': correct,
            'precision': correct / len(accepted) if accepted else None,
            'coverage': len(accepted) / len(rows) if rows else None,
            'abstentions': len(rows) - len(accepted),
            'unavailable': sum(r[key] is None for r in rows),
            'wrong_accepted': [r['id'] for r in accepted if r[key]['category'] != r['expected']]}


def evaluate(catalog_path, output, live=False):
    catalog, raw = load_catalog(catalog_path)
    output.mkdir(parents=True, exist_ok=False)
    write(output / 'catalog.json', catalog)
    runtime = Path(__file__).parent.parent / 'failure_triage'
    write(output / 'freeze.json', {
        'created_at': datetime.now(timezone.utc).isoformat(), 'catalog_sha256': digest(raw),
        'runtime_sha256': {p.name: digest(p.read_bytes()) for p in sorted(runtime.glob('*.py'))},
        'shared_transport_sha256': digest((runtime.parent/'decisions_transport.py').read_bytes()),
        'max_calls': len(catalog['cases']), 'max_estimated_usd': MAX_USD,
        'reserve_usd_per_call': RESERVE_USD, 'label_policy': 'frozen before inference; observed failure family, not proven root cause'})
    # Freeze every prepared snapshot before ANY network request, including pricing.
    for case in catalog['cases']:
        directory = output / case['id']
        directory.mkdir()
        manifest = directory / 'manifest.json'
        command('import-report', '--kind', case['report_kind'], '--input', case['report']['path'],
                '--run-id', case['run_id'], '--output', str(manifest))
        snapshot = build_request(manifest, [f'{s["path"]}:{s["start"]}:{s["end"]}' for s in case['logs']])
        # Verify bytes actually embedded, not just the earlier filesystem read.
        if [s['sha256'] for s in snapshot['state']['excerpts']] != [s['sha256'] for s in case['logs']]:
            raise RequestError('evidence changed during preparation')
        if snapshot['state']['facts']['source_report']['sha256'] != case['report']['sha256']:
            raise RequestError('report changed during preparation')
        (directory/'request.json').write_bytes(serialize_json(snapshot))
        command('diagnose', '--request', str(directory/'request.json'), '--output', str(directory/'baseline.json'))
    if live:
        write(output/'pricing.json', price_check())
    rows, calls, spent, stop = [], 0, 0.0, None
    for case in catalog['cases']:
        directory = output/case['id']
        baseline, _, _ = read_json_file(directory/'baseline.json', limit=200_000, label='baseline')
        result = None
        if live:
            if calls >= MAX_CASES or spent + RESERVE_USD > MAX_USD:
                stop = 'reservation_limit'
                break
            calls += 1
            write(output/'progress.json', {'calls': calls, 'reported_cost_usd': spent, 'rows': rows,
                                         'pending_attempt': case['id'], 'stop': 'pending_unknown_usage'})
            try:
                exit_code = command('diagnose', '--request', str(directory/'request.json'),
                                    '--output', str(directory/'live.json'), '--live')
                result, _, _ = read_json_file(directory/'live.json', limit=200_000, label='live report')
                cost = (result.get('usage') or {}).get('cost')
                if type(cost) not in (int, float) or not math.isfinite(cost) or cost < 0:
                    stop = 'unknown_usage'
                else:
                    spent += cost
                    if cost > RESERVE_USD:
                        stop = 'cost_exceeds_reservation'
                if exit_code != 0 or result.get('status') != 'live' or result.get('fallback'):
                    result = None
                    stop = stop or 'unavailable_response'
            except (OSError, ValueError, TypeError, AttributeError, RuntimeError, subprocess.TimeoutExpired):
                stop, result = 'attempt_failed_unknown_usage', None
        rows.append({'id': case['id'], 'cohort': case['cohort'], 'expected': case['expected'],
                     'baseline': baseline['hypothesis'], 'jev': result['hypothesis'] if result else None,
                     'raw_model_hypothesis': result.get('model_hypothesis') if result else None,
                     'request_sha256': baseline['request_sha256'],
                     'facts_preserved': result is None or serialize_json(result['facts']) == serialize_json(baseline['facts']),
                     'latency_ms': result['latency_ms'] if result else None,
                     'usage': result['usage'] if result else None})
        write(output/'progress.json', {'calls': calls, 'reported_cost_usd': spent, 'rows': rows, 'stop': stop})
        print(case['id'] + ': retained', flush=True)
        if stop:
            break
    summary = {'origin': catalog.get('origin'), 'calls': calls, 'reported_cost_usd': spent, 'stop': stop,
               'facts_preserved': all(r['facts_preserved'] for r in rows),
               'cohorts': {cohort: {key: metrics([r for r in rows if r['cohort'] == cohort], key)
                                   for key in ('baseline', 'jev') if key == 'baseline' or live}
                           for cohort in ('failure', 'control')}}
    write(output/'summary.json', summary)
    return 2 if stop else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--live', action='store_true')
    args = parser.parse_args()
    try:
        return evaluate(args.cases, args.output, args.live)
    except (RequestError, OSError, ValueError, RuntimeError) as error:
        print(f'evaluation failed: {type(error).__name__}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
