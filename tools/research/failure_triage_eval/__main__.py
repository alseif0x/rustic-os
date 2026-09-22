# SPDX-License-Identifier: Apache-2.0
"""Run frozen synthetic cases through the ordinary CLI, never through a test runner."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
import urllib.request

from .cases import cases

PRICE_URL = 'https://openrouter.ai/api/v1/models/typesafe/jev-1.13/endpoints'
MAX_CALLS = 50
RESERVE_USD = 0.01  # Above 48,000 input tokens at the checked price, no output charge.
MAX_USD = 1.0


def write(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n', encoding='utf-8')


def price_check():
    with urllib.request.urlopen(PRICE_URL, timeout=20) as response:
        raw = response.read(100_001)
    if len(raw) > 100_000:
        raise ValueError('pricing response too large')
    data = json.loads(raw)
    endpoints = data['data']['endpoints']
    if not endpoints or any(float(e['pricing']['prompt']) != 0.000000042
                            or float(e['pricing']['completion']) != 0 for e in endpoints):
        raise ValueError('pricing changed; review reservation before live evaluation')
    return data


def command(*args):
    result = subprocess.run([sys.executable, '-m', 'tools.research.failure_triage', *args],
                            capture_output=True, text=True, timeout=45)
    if result.returncode not in (0, 2):
        raise RuntimeError('triage CLI failed; inspect selected inputs (no raw stderr copied)')
    return result.returncode


def metrics(rows, field):
    selected = [r for r in rows if r.get(field) and r[field]['accepted']]
    correct = sum(r[field]['category'] == r['expected'] for r in selected)
    # Wilson interval: small fixture counts are not a population accuracy claim.
    n = len(selected)
    interval = None
    if n:
        p, z = correct / n, 1.96
        center = (p + z*z/(2*n)) / (1+z*z/n)
        half = z * math.sqrt(p*(1-p)/n + z*z/(4*n*n)) / (1+z*z/n)
        interval = [center-half, center+half]
    eligible = [r for r in rows if r['expected'] != 'unknown']
    return {'cases': len(rows), 'accepted': n, 'correct_accepted': correct,
            'precision': correct/n if n else None,
            'wilson_95_interval': interval,
            'coverage': n/len(rows) if rows else None,
            'abstentions': len(rows)-n,
            'unavailable': sum(r.get(field) is None for r in rows),
            'top3_evidence_hits': sum(bool(r.get(field) and r[field]['top3_hit']) for r in eligible),
            'top3_evidence_eligible': len(eligible),
            'wrong_accepted_ids': [r['id'] for r in selected if r[field]['category'] != r['expected']]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--live', action='store_true')
    parser.add_argument('--split', choices=['all', 'development', 'heldout'], default='all')
    args = parser.parse_args()
    # Refuse overwriting prior evidence, including interrupted evaluations.
    args.output.mkdir(parents=True, exist_ok=False)
    dataset = [c for c in cases() if args.split == 'all' or c['split'] == args.split]
    frozen = json.dumps(dataset, sort_keys=True, separators=(',', ':')).encode()
    write(args.output / 'labels.json', dataset)
    write(args.output / 'freeze.json', {'origin': 'original synthetic fixtures',
          'created_at': datetime.now(timezone.utc).isoformat(),
          'shared_transport_sha256': hashlib.sha256(
              Path(__file__).parent.parent.joinpath('decisions_transport.py').read_bytes()).hexdigest(),
          'dataset_sha256': hashlib.sha256(frozen).hexdigest(),
          'fixture_source_sha256': hashlib.sha256(Path(__file__).with_name('cases.py').read_bytes()).hexdigest(),
          'implementation_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
              for p in sorted(Path(__file__).parent.parent.joinpath('failure_triage').glob('*.py'))},
          'max_calls': MAX_CALLS, 'max_estimated_usd': MAX_USD,
          'reserve_usd_per_call': RESERVE_USD, 'split': args.split})
    if args.live:
        write(args.output / 'pricing.json', price_check())
    rows, calls, spent = [], 0, 0.0
    stop = None
    for case in dataset:
        folder = args.output / case['id']
        folder.mkdir()
        manifest = folder / 'manifest.json'
        write(manifest, case['manifest'])
        argv = ['prepare', '--manifest', str(manifest)]
        for index, log in enumerate(case['logs'], 1):
            source = folder / f'excerpt-{index}.log'
            source.write_text(log, encoding='utf-8')
            argv += ['--log', f'{source}:1:{len(log.splitlines())}']
        request = folder / 'request.json'
        command(*argv, '--output', str(request))
        command('diagnose', '--request', str(request), '--output', str(folder / 'baseline.json'))
        baseline = json.loads((folder / 'baseline.json').read_text())
        live = None
        if args.live:
            if calls >= MAX_CALLS or spent + RESERVE_USD > MAX_USD:
                stop = 'call_or_reservation_limit'
                break
            calls += 1
            write(args.output / 'progress.json', {'calls': calls, 'reported_cost_usd': spent,
                  'rows': rows, 'pending_attempt': case['id'], 'stop': 'pending_unknown_usage'})
            try:
                exit_code = command('diagnose', '--request', str(request),
                                    '--output', str(folder / 'live.json'), '--live')
                live = json.loads((folder / 'live.json').read_text())
                usage = live.get('usage') or {}
                cost = usage.get('cost')
                if type(cost) not in (float, int) or not math.isfinite(cost) or cost < 0:
                    stop = 'unknown_usage_or_failed_call'
                else:
                    spent += cost
                    if cost > RESERVE_USD:
                        stop = 'cost_exceeds_reservation'
                if exit_code != 0 or live.get('fallback') or live.get('status') != 'live':
                    live = None  # Retain live.json, but never score fallback as JEV.
                    stop = stop or 'unavailable_response'
            except (OSError, ValueError, TypeError, AttributeError, RuntimeError, subprocess.TimeoutExpired):
                live = None
                stop = 'attempt_failed_unknown_usage'
        relevant = f'excerpt-{case["relevant_excerpt"]:03d}'
        def scored(report):
            hypothesis = report['hypothesis']
            return {**hypothesis, 'top3_hit': relevant in [r['id'] for r in report['evidence'][:3]]}
        row = {'id': case['id'], 'split': case['split'], 'expected': case['expected'],
               'baseline': scored(baseline), 'jev': scored(live) if live else None,
               'latency_ms': live.get('latency_ms') if live else None,
               'usage': live.get('usage') if live else None,
               'raw_model_hypothesis': live.get('model_hypothesis') if live else None,
               'model_failure_on_passed_harness': bool(live and
                   case['manifest']['facts'].get('harness_passed') is True and
                   live.get('model_hypothesis', {}).get('accepted') is True),
               'facts_preserved': baseline['facts'] == case['manifest']['facts'] and
                                  (live is None or live['facts'] == case['manifest']['facts'])}
        rows.append(row)
        write(args.output / 'progress.json', {'calls': calls, 'reported_cost_usd': spent, 'rows': rows, 'stop': stop})
        print(f'{case["id"]}: report retained', flush=True)
        if stop:
            break
    latencies = sorted(r['latency_ms'] for r in rows if type(r['latency_ms']) in (int, float))
    latency = {f'p{p}_ms': latencies[max(0, math.ceil(len(latencies)*p/100)-1)]
               if latencies else None for p in (50, 95)}
    summary = {'latency_successful_calls': latency, 'origin': 'synthetic; not representative historical failures',
               'calls': calls, 'reported_cost_usd': spent, 'stop': stop,
               'facts_preserved': all(r['facts_preserved'] for r in rows),
               'model_failure_hypotheses_on_passed_harness':
                   sum(r['model_failure_on_passed_harness'] for r in rows), 'splits': {}}
    for split in ['development', 'heldout']:
        group = [r for r in rows if r['split'] == split]
        summary['splits'][split] = {'baseline': metrics(group, 'baseline')}
        if args.live:
            summary['splits'][split]['jev'] = metrics(group, 'jev')
    write(args.output / 'summary.json', summary)
    return 2 if stop else 0


if __name__ == '__main__':
    raise SystemExit(main())
