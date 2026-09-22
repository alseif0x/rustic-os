# SPDX-License-Identifier: Apache-2.0
"""Historical evidence provenance and paid-attempt accounting contracts."""
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tools.research.failure_triage.format import RequestError
from tools.research.failure_triage_eval import historical as evaluation

CATALOG = Path('tools/research/failure_triage_eval/historical/cases.json')


class HistoricalTriageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.catalog = json.loads(CATALOG.read_text())
        self.catalog['cases'] = self.catalog['cases'][:1]
        self.path = self.root/'cases.json'
        self.save()

    def tearDown(self):
        self.temp.cleanup()

    def save(self):
        self.path.write_text(json.dumps(self.catalog))

    def test_changed_native_report_or_excerpt_is_refused(self):
        for field in ('report', 'log'):
            with self.subTest(field=field):
                changed = copy.deepcopy(self.catalog)
                target = changed['cases'][0]['report'] if field == 'report' else changed['cases'][0]['logs'][0]
                target['sha256'] = '0'*64
                self.path.write_text(json.dumps(changed))
                with self.assertRaises(RequestError):
                    evaluation.load_catalog(self.path)

    def test_duplicates_and_output_path_escape_are_refused(self):
        self.catalog['cases'].append(copy.deepcopy(self.catalog['cases'][0]))
        self.save()
        with self.assertRaises(RequestError):
            evaluation.load_catalog(self.path)
        self.catalog['cases'] = self.catalog['cases'][:1]
        self.catalog['cases'][0]['id'] = '../escape'
        self.save()
        with self.assertRaises(RequestError):
            evaluation.load_catalog(self.path)

    def test_renamed_duplicate_native_report_is_refused(self):
        duplicate = copy.deepcopy(self.catalog['cases'][0])
        duplicate.update(id='renamed-case', incident_group='renamed-group', run_id='renamed-run')
        self.catalog['cases'].append(duplicate)
        self.save()
        with self.assertRaisesRegex(RequestError, 'duplicate native report'):
            evaluation.load_catalog(self.path)

    def test_offline_retains_evidence_without_transmitting_labels(self):
        output = self.root/'offline'
        with mock.patch.object(evaluation, 'price_check', side_effect=AssertionError('network')) as price:
            self.assertEqual(evaluation.evaluate(self.path, output), 0)
        price.assert_not_called()
        case = self.catalog['cases'][0]
        request = json.loads((output/case['id']/'request.json').read_text())
        self.assertNotIn('expected', request['state'])
        self.assertNotIn(case['label_basis'], json.dumps(request))
        self.assertEqual(request['state']['facts']['source_report']['sha256'], case['report']['sha256'])
        self.assertEqual([e['sha256'] for e in request['state']['excerpts']], [e['sha256'] for e in case['logs']])

    def test_failed_live_attempt_is_retained_but_never_scored_as_jev(self):
        for failure in ('timeout', 'malformed', 'fallback'):
            with self.subTest(failure=failure):
                output = self.root/failure
                real_command = evaluation.command
                case_id = self.catalog['cases'][0]['id']

                def invoke(*args):
                    if '--live' not in args:
                        return real_command(*args)
                    pending = json.loads((output/'progress.json').read_text())
                    self.assertEqual(pending['calls'], 1)
                    self.assertEqual(pending['pending_attempt'], case_id)
                    target = Path(args[args.index('--output')+1])
                    if failure == 'timeout':
                        raise subprocess.TimeoutExpired('fake', 45)
                    if failure == 'malformed':
                        target.write_text('{')
                        return 0
                    baseline = json.loads((output/case_id/'baseline.json').read_text())
                    baseline.update(status='unavailable', fallback='baseline', usage=None)
                    target.write_text(json.dumps(baseline))
                    return 2

                def price():
                    self.assertTrue((output/case_id/'request.json').exists())
                    self.assertTrue((output/'freeze.json').exists())
                    return {}

                with mock.patch.object(evaluation, 'command', side_effect=invoke), \
                     mock.patch.object(evaluation, 'price_check', side_effect=price):
                    self.assertEqual(evaluation.evaluate(self.path, output, live=True), 2)
                summary = json.loads((output/'summary.json').read_text())
                self.assertEqual(summary['calls'], 1)
                self.assertEqual(summary['cohorts']['failure']['jev']['accepted'], 0)
                self.assertEqual(summary['cohorts']['failure']['jev']['unavailable'], 1)
                self.assertIsNotNone(summary['stop'])


if __name__ == '__main__':
    unittest.main()
