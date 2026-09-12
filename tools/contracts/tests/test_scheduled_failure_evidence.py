# SPDX-License-Identifier: Apache-2.0
"""Challenge each scheduled failure claim, including raw effect and reply-loss boundaries."""
import copy
import unittest
from tools.contracts.catalog import Catalog
from tools.contracts.scheduled_failure_evidence import check_scheduled_failures
from tools.contracts.validation import ContractError
from tools.contracts.tests.scheduled_failure_fixtures import failure_cases


class ScheduledFailureEvidence(unittest.TestCase):
    def setUp(self):
        self.catalog = Catalog()

    def rejected(self, index, mutation):
        cases = failure_cases()
        mutation(cases[index])
        with self.assertRaises(ContractError):
            check_scheduled_failures(self.catalog, cases)

    def test_complete_failure_profile_and_exact_inventory(self):
        cases = failure_cases()
        self.assertEqual(check_scheduled_failures(self.catalog, cases), 9)
        for broken in (cases[:-1], cases + cases[:1], cases[:1] + cases[:1] + cases[2:]):
            with self.assertRaises(ContractError): check_scheduled_failures(self.catalog, broken)
        self.rejected(0, lambda c: c.update(case='scheduled_failure_unknown'))

    def test_every_cut_requires_native_identity_observations_and_reboot(self):
        for i in range(9):
            for mutate in (lambda c: c.update(verified=False), lambda c: c.update(reboot_verified=False),
                           lambda c: c.update(no_replay=False), lambda c: c.update(sha256='invalid'),
                           lambda c: c['observations'].pop(), lambda c: c['durable'].pop(),
                           lambda c: c['durable'][1].update(id=c['durable'][0]['id']),
                           lambda c: c['observations'][0].update(pending=True),
                           lambda c: c['observations'][2].update(phase='running')):
                self.rejected(i, mutate)

    def test_late_stop_false_rollback_and_unrelated_receipt_are_rejected(self):
        for i in (0, 1, 2, 3, 5, 7):
            self.rejected(i, lambda c: c['completions'].clear())
            self.rejected(i, lambda c: c['completions'][0]['receipt'].update(sha256='0'*64))
            self.rejected(i, lambda c: c['completions'][0].update(operation_id='unrelated'))
            self.rejected(i, lambda c: c['completions'][0]['receipt'].update(previous_version='v_0000000000000001'))
        for i in (0, 1):
            self.rejected(i, lambda c: c['durable'][0].update(state='cancelled'))
            self.rejected(i, lambda c: c.update(skip=0))
            self.rejected(i, lambda c: c['observations'][-1].update(phase='stopping'))

    def test_lost_response_restart_and_pressure_require_the_actual_boundary(self):
        for i in (2, 3):
            self.rejected(i, lambda c: c.update(discarded_reply=False))
            self.rejected(i, lambda c: c.update(stale_reply_rejected=False))
        self.rejected(3, lambda c: c.update(terminal_reply_ready=False))
        for i in (4, 5):
            self.rejected(i, lambda c: c.update(new_service_pid=c['old_service_pid']))
            self.rejected(i, lambda c: c.update(new_service_pid=True))
            self.rejected(i, lambda c: c.update(restart_drained_io=False))
        for field in ('retained_full', 'staging_full', 'undrained_client', 'staging_retained', 'owner_progress', 'reclaimed'):
            self.rejected(8, lambda c, f=field: c.update({f: False}))

    def test_uncertain_settlement_cannot_invent_prevention_or_resume_pending_work(self):
        for i in (6, 7):
            self.rejected(i, lambda c: c.update(query_error='Cancelled'))
            self.rejected(i, lambda c: c.update(pending_abandoned=False))
            self.rejected(i, lambda c: c.update(reconciled_after_restart=False))
            self.rejected(i, lambda c: c['durable'][1].update(state='committed', terminal=6))
        self.rejected(6, lambda c: c['durable'][0].update(state='cancelled', terminal=5))

    def test_independent_file_content_and_versions_must_agree_with_the_effect(self):
        for i in range(9):
            for mutate in (lambda c: c.update(file_sha256='0'*64), lambda c: c.update(file_size=True),
                           lambda c: c.update(file_version='v_0000000000000099'),
                           lambda c: c.update(previous_version='v_0')):
                self.rejected(i, mutate)
