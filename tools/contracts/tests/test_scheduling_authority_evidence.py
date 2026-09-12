# SPDX-License-Identifier: Apache-2.0
import copy
import unittest
from tools.contracts.catalog import Catalog
from tools.contracts.validation import ContractError
from tools.contracts.scheduling_authority_evidence import check_authority
from tools.contracts.tests.scheduling_authority_fixtures import authority_cases


class SchedulingAuthorityEvidence(unittest.TestCase):
    def setUp(self):
        self.catalog = Catalog()

    def reject(self, change):
        cases = authority_cases()
        change(cases)
        with self.assertRaises(ContractError):
            check_authority(self.catalog, cases)

    def test_complete_synthetic_matrix_is_accepted(self):
        self.assertEqual(check_authority(self.catalog, authority_cases()), 5)

    def test_inventory_and_verification_are_mandatory(self):
        self.reject(lambda c: c.pop())
        self.reject(lambda c: c.__setitem__(1, copy.deepcopy(c[0])))
        for flag in ('verified', 'reboot_verified', 'no_replay', 'reclaimed', 'owner_progress'):
            self.reject(lambda c: c[0].__setitem__(flag, False))

    def test_missing_or_aliased_principals_and_authority_are_rejected(self):
        for field, value in (('rights', 15), ('scope', 'other'), ('pid', 1), ('subject', 8), ('private', 1), ('revoked', True)):
            self.reject(lambda c: c[0]['actors'][0].__setitem__(field, value))
        self.reject(lambda c: c[2]['actors'][1].__setitem__('subject', 1))
        self.reject(lambda c: c[0]['actors'][1].__setitem__('pid', c[0]['actors'][0]['pid']))

    def test_denials_must_be_complete_nonleaking_and_cover_both_tickets(self):
        self.reject(lambda c: c[0]['actors'][0]['attempts'].pop())
        self.reject(lambda c: c[0]['actors'][0]['attempts'][4].__setitem__('index', 1))
        self.reject(lambda c: c[0]['actors'][0]['initial'].__setitem__('status', 0))
        for field, value in (('status', 0), ('value', 1), ('other', 1), ('control_denied', 1), ('version', 9)):
            self.reject(lambda c: c[0]['actors'][0]['attempts'][3].__setitem__(field, value))
        self.reject(lambda c: c[3]['actors'][1]['attempts'][4].__setitem__('other', 0))

    def test_refused_calls_cannot_latch_a_stop_or_change_committed_bytes(self):
        self.reject(lambda c: c[0]['observations'][3].__setitem__('requested', 1))
        self.reject(lambda c: c[0]['observations'][3].__setitem__('pending', 0))
        self.reject(lambda c: c[0]['durable'][0].__setitem__('state', 'cancelled'))
        self.reject(lambda c: c[0]['completions'][0].__setitem__('operation_id', 'unrelated'))
        self.reject(lambda c: c[0].__setitem__('file_sha256', '0'*64))

    def test_human_edit_requires_changed_version_conflict_and_stable_admission(self):
        for field, value in (('conflict', 'Busy'), ('conflict_unchanged', False),
                             ('file_version', 'v_0000000000000002'), ('file_size', 5)):
            self.reject(lambda c: c[4].__setitem__(field, value))
        self.reject(lambda c: c[4]['admitted'].__setitem__('instance', 'si_' + '09'*16 + '_0000000000000003'))
        self.reject(lambda c: c[4]['durable'][0].__setitem__('state', 'committed'))
        self.reject(lambda c: c[4].__setitem__('completions', c[0]['completions']))
