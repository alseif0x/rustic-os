# SPDX-License-Identifier: Apache-2.0
import unittest
from copy import deepcopy
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.prevention_report import verify, CAUSE


def fixture():
    def pair(numbers, state, causes):
        views, clients = [], []
        for number, cause in zip(numbers, causes, strict=True):
            terminal = 0 if state == 'admitted' else number+1
            views.append(dict(profile=2,kind='retained',id=f"ad_{'07'*16}_{number:016x}",
                              instance=f"si_{'07'*16}_0000000000000001",state=state,
                              terminal=terminal,prevention=cause))
            clients.append(dict(status=0,value=1 if state=='admitted' else 2,other=terminal,
                                control_denied=0,version=CAUSE[cause]))
        return dict(views=views,clients=clients)
    legacy = pair([2], 'cancelled', ['unknown'])
    terminal = pair([6,8], 'cancelled', ['requested','version_conflict'])
    authority_pair = pair([4], 'cancelled', ['authority_lost'])
    retained = authority_pair['views'][0]
    authority = dict(retained=deepcopy(retained),paired=authority_pair,
                     live=dict(profile=2,kind='active',id=retained['id'],instance=retained['instance'],phase='running',pending=1,requested=0),
                     live_client=dict(status=0,value=17,other=0,control_denied=1,version=0),
                     cancel_only=dict(status=17,value=0,other=0,control_denied=0,version=0),
                     revoked=dict(status=18,value=0,other=0,control_denied=0,version=0))
    return dict(verified=True,format=5,replay_writes=0,legacy_cause='unknown',reasons=['requested','version_conflict'],
                results=[dict(id=v['id'],instance=v['instance'],state='cancelled',terminal=v['terminal'],number=n) for v,n in zip(terminal['views'],[6,8],strict=True)],
                observations=dict(legacy=legacy,migrated=deepcopy(legacy),authority=authority,
                                  prepared=pair([6,8],'admitted',['none','none']),terminal=terminal,reboot=deepcopy(terminal)))


class PreventionReport(unittest.TestCase):
    def test_complete_report_and_contradictory_history(self):
        self.assertTrue(verify(fixture()))
        for group in ['legacy','migrated','prepared','terminal','reboot']:
            for key,value in [('profile',1),('terminal',0),('prevention','authority_lost'),('id','malformed')]:
                report = fixture()
                if report['observations'][group]['views'][0][key] == value:
                    continue
                report['observations'][group]['views'][0][key] = value
                with self.assertRaises(AssertionError):
                    verify(report)

    def test_missing_comparisons_and_denial_data_cannot_pass(self):
        for group in ['legacy','prepared','terminal']:
            report = fixture()
            report['observations'][group]['clients'].clear()
            with self.assertRaises(AssertionError):
                verify(report)
        for denial in ['cancel_only','revoked']:
            for key in ['status','value','other','control_denied','version']:
                report = fixture()
                report['observations']['authority'][denial][key] += 1
                with self.assertRaises(AssertionError):
                    verify(report)

    def test_live_cause_fabrication_and_misbound_results_are_rejected(self):
        for mutate in [lambda r:r['observations']['authority']['live'].update(prevention='requested'),
                       lambda r:r['observations']['authority']['live'].update(pending=True),
                       lambda r:r['observations']['reboot']['clients'][0].update(control_denied=False),
                       lambda r:r['results'][0].update(number=99),
                       lambda r:r['observations']['authority']['retained'].update(instance='si_'+'08'*16+'_0000000000000001')]:
            report = fixture()
            mutate(report)
            with self.assertRaises(AssertionError):
                verify(report)
