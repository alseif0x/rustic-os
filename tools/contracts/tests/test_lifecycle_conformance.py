# SPDX-License-Identifier: Apache-2.0
import copy
import unittest
from jsonschema import Draft202012Validator
from tools.contracts.lifecycle_conformance import catalog, host_check, validate_exchange
from tools.contracts.validation import ContractError


class Lifecycle(unittest.TestCase):
    def setUp(self):
        self.catalog = catalog()
        self.id = 'ad_' + '07' * 16 + '_0000000000000009'
        self.request = dict(version=2, method='operations.get', params=dict(operation_id=self.id))
        self.response = dict(version=2, method='operations.get', result=dict(
            operation_id=self.id, service_instance='si_' + '07' * 16 + '_0000000000000008',
            state='succeeded', effect='committed', completion_id='op_' + '07' * 16 + '_000000000000000c'))

    def test_all_states_and_minimal_ack_shapes(self):
        self.assertEqual(host_check(self.catalog)['exchanges'], 11)
        self.assertFalse(host_check(self.catalog)['guest_execution'])
        validate_exchange(self.catalog, self.request, self.response)
        for descriptor in self.catalog.descriptors():
            self.assertEqual(descriptor['version'], 2)
            self.assertEqual(len(descriptor['contract_sha256']), 64)
            Draft202012Validator.check_schema(descriptor['responseSchema'])

    def test_completed_identity_cannot_change_or_cross_lineage(self):
        for key, value in [('operation_id', self.response['result']['completion_id']),
                           ('completion_id', 'op_' + '08' * 16 + '_000000000000000c'),
                           ('completion_id', 'op_' + '07' * 16 + '_0000000000000009'),
                           ('service_instance', 'si_' + '07' * 16 + '_000000000000000a'),
                           ('service_instance', 'si_' + '07' * 16 + '_0000000000000000')]:
            response = copy.deepcopy(self.response)
            response['result'][key] = value
            with self.assertRaises(ContractError):
                validate_exchange(self.catalog, self.request, response)

    def test_no_terminal_stop_history_or_fabricated_legacy_cancellation(self):
        for field, value in [('cancel_requested', False), ('stop_pending', True), ('receipt', {})]:
            response = copy.deepcopy(self.response)
            response['result'][field] = value
            with self.assertRaises(ContractError):
                validate_exchange(self.catalog, self.request, response)
        result = self.response['result']
        del result['completion_id']
        result.update(state='prevented', effect='none')
        validate_exchange(self.catalog, self.request, self.response)
        result['failure'] = 'requested'
        with self.assertRaises(ContractError):
            validate_exchange(self.catalog, self.request, self.response)

    def test_cancel_only_cannot_return_an_operation_or_receipt(self):
        request = dict(version=2, method='operations.cancel', params=dict(operation_id=self.id))
        response = dict(version=2, method='operations.cancel', result=dict(operation_id=self.id, disposition='too_late'))
        validate_exchange(self.catalog, request, response)
        for field, value in [('operation', self.response['result']), ('service_instance', self.response['result']['service_instance']), ('state', 'succeeded')]:
            changed = copy.deepcopy(response)
            changed['result'][field] = value
            with self.assertRaises(ContractError):
                validate_exchange(self.catalog, request, changed)

    def test_live_reconciling_cannot_claim_an_effect_or_historical_receipt(self):
        result = self.response['result']
        del result['completion_id']
        result.update(state='reconciling', effect='unknown', stop_pending=True)
        validate_exchange(self.catalog, self.request, self.response)
        for key, value in [('effect', 'none'), ('stop_pending', 1), ('state', 'prepared')]:
            changed = copy.deepcopy(self.response)
            changed['result'][key] = value
            with self.assertRaises(ContractError):
                validate_exchange(self.catalog, self.request, changed)
