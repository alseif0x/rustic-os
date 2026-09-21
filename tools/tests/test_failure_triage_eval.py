# SPDX-License-Identifier: Apache-2.0
"""Evaluation accounting must not count abstentions as correct predictions."""

import unittest
import json
from pathlib import Path
import subprocess
import tempfile
from unittest import mock

from tools.research.failure_triage_eval import __main__ as evaluation

from tools.research.failure_triage_eval.__main__ import metrics
from tools.research.failure_triage_eval.cases import cases


class TriageEvaluationTest(unittest.TestCase):
    def test_abstention_and_wrong_acceptance_accounting(self):
        rows = [
            {'id': 'one', 'expected': 'timeout', 'baseline': {'category': 'timeout', 'accepted': True, 'top3_hit': True}},
            {'id': 'two', 'expected': 'unknown', 'baseline': {'category': 'unknown', 'accepted': False, 'top3_hit': True}},
            {'id': 'three', 'expected': 'assertion', 'baseline': {'category': 'compilation', 'accepted': True, 'top3_hit': False}},
        ]
        result = metrics(rows, 'baseline')
        self.assertEqual(result['accepted'], 2)
        self.assertEqual(result['correct_accepted'], 1)
        self.assertEqual(result['precision'], 0.5)
        self.assertEqual(result['coverage'], 2/3)
        self.assertEqual(result['abstentions'], 1)
        self.assertEqual(result['wrong_accepted_ids'], ['three'])
        self.assertEqual(result['top3_evidence_hits'], 1)
        self.assertEqual(result['top3_evidence_eligible'], 2)

    def test_fixture_split_and_nontrivial_evidence_selection(self):
        dataset = cases()
        self.assertEqual(len(dataset), 50)
        self.assertEqual(sum(c['split'] == 'heldout' for c in dataset), 20)
        self.assertEqual(len({c['id'] for c in dataset}), 50)
        self.assertEqual({c['origin'] for c in dataset}, {'synthetic'})
        self.assertEqual({c['relevant_excerpt'] for c in dataset}, {1, 2, 3, 4, 5})
        for case in dataset:
            self.assertEqual(len(case['logs']), 5)
            self.assertNotIn('expected', case['manifest'])

    def test_live_failure_never_scores_fallback_and_persists_attempt(self):
        for failure in ('fallback', 'timeout', 'malformed'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / 'evaluation'
                fixture = cases()[0]
                report = {'facts': fixture['manifest']['facts'],
                          'hypothesis': {'category': fixture['expected'], 'accepted': True, 'confidence': 1},
                          'evidence': [{'id': f'excerpt-{fixture["relevant_excerpt"]:03d}'}]}

                def invoke(*args):
                    target = Path(args[args.index('--output') + 1])
                    if args[0] == 'prepare':
                        evaluation.write(target, {})
                    elif '--live' not in args:
                        evaluation.write(target, report)
                    else:
                        pending = json.loads((output / 'progress.json').read_text())
                        self.assertEqual(pending['calls'], 1)
                        self.assertEqual(pending['pending_attempt'], fixture['id'])
                        if failure == 'timeout':
                            raise subprocess.TimeoutExpired('fake', 45)
                        if failure == 'malformed':
                            target.write_text('{')
                        else:
                            evaluation.write(target, {**report, 'fallback': True, 'usage': None})
                            return 2
                    return 0

                with mock.patch.object(evaluation, 'cases', return_value=[fixture]), \
                     mock.patch.object(evaluation, 'command', side_effect=invoke), \
                     mock.patch.object(evaluation, 'price_check', return_value={}), \
                     mock.patch('sys.argv', ['evaluation', '--output', str(output), '--live']):
                    self.assertEqual(evaluation.main(), 2)
                summary = json.loads((output / 'summary.json').read_text())
                self.assertEqual(summary['calls'], 1)
                self.assertEqual(summary['splits']['development']['jev']['accepted'], 0)
                self.assertEqual(summary['splits']['development']['jev']['unavailable'], 1)
                self.assertIsNotNone(summary['stop'])


if __name__ == '__main__':
    unittest.main()
