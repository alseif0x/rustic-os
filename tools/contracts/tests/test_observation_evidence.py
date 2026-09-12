# SPDX-License-Identifier: Apache-2.0
import copy
import unittest
from tools.contracts.catalog import Catalog
from tools.contracts.scheduling_evidence import check_scheduling
from tools.contracts.tests.scheduling_fixtures import scheduling_cases
from tools.contracts.validation import ContractError


class ObservationEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.catalog = Catalog()

    def rejects(self, mutations):
        for mutate in mutations:
            cases = scheduling_cases()
            mutate(cases)
            with self.assertRaises(ContractError):
                check_scheduling(self.catalog, cases)

    def test_missing_duplicate_or_stale_observations_are_rejected(self):
        self.rejects((lambda c: c[0].pop('coherent_observations'),
                      lambda c: c[0]['coherent_observations'].pop(),
                      lambda c: c[0]['coherent_observations'].__setitem__(2, copy.deepcopy(c[0]['coherent_observations'][6])),
                      lambda c: c[0].__setitem__('observation_read_only', False)))

    def test_profile_identity_and_unconfirmed_effects_cannot_be_fabricated(self):
        for index in range(10):
            for key, bad in (('profile', 2), ('profile', True), ('id', 'op_wrong'),
                             ('instance', 'si_wrong'), ('effect', 'committed')):
                self.rejects((lambda c, i=index, k=key, v=bad: c[0]['coherent_observations'][i].__setitem__(k, v),))
        self.rejects((lambda c: c[0]['coherent_observations'][5].__setitem__('requested', 0),
                      lambda c: c[0]['coherent_observations'][2].__setitem__('pending', False),
                      lambda c: c[0]['coherent_observations'][6].__setitem__('terminal', 0)))

    def test_restart_does_not_claim_a_ticket_or_replace_its_identity(self):
        self.rejects((lambda c: c[2].pop('coherent_observations'),
                      lambda c: c[2]['coherent_observations'][0].__setitem__('state', 'queued'),
                      lambda c: c[2]['coherent_observations'][1].__setitem__('terminal', 6),
                      lambda c: c[2]['coherent_observations'][2].__setitem__('instance', 'si_other')))

    def test_deterministic_parity_and_empty_denials_are_required(self):
        self.rejects((lambda c: c[0]['observation_clients'].pop(),
                      lambda c: c[0]['observation_clients'][0].__setitem__('index', 2),
                      lambda c: c[0]['observation_clients'][1]['result'].__setitem__('value', 1),
                      lambda c: c[0]['observation_denied'].__setitem__('other', 3),
                      lambda c: c[0]['observation_denied'].__setitem__('status', 0),
                      lambda c: c[3]['observation_revoked'].__setitem__('status', 3)))
