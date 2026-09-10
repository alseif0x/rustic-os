# SPDX-License-Identifier: Apache-2.0
import copy
import json
import unittest

from jsonschema import Draft202012Validator

from tools.contracts.__main__ import check
from tools.contracts.catalog import Catalog
from tools.contracts.validation import ContractError, parse_message, validate_exchange


class Contracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.catalog = Catalog()
        cls.cases = json.loads((cls.catalog.root / "fixtures/messages.json").read_text())
        cls.exchanges = json.loads((cls.catalog.root / "fixtures/exchanges.json").read_text())

    def exchange(self, method):
        return copy.deepcopy(next(item for item in self.exchanges
                                  if item["request"]["method"] == method))

    def reject_exchange(self, exchange):
        with self.assertRaises(ContractError):
            validate_exchange(self.catalog, exchange["request"], exchange["response"])

    def test_reviewed_messages_and_exchanges(self):
        check(self.catalog)

    def test_descriptors_preserve_schema_acceptance(self):
        descriptors = {item["name"]: item for item in self.catalog.descriptors()}
        for case in self.cases:
            message = case["message"]
            method = message["method"]
            if case["direction"] == "request":
                comparisons = [("input", "inputSchema", message["params"])]
            else:
                comparisons = [("response", "responseSchema", message)]
                if "result" in message:
                    comparisons.append(("output", "outputSchema", message["result"]))
            for part, exported, value in comparisons:
                native = self.catalog.validator(method, part).is_valid(value)
                descriptor = descriptors[method][exported]
                self.assertEqual(native, Draft202012Validator(descriptor).is_valid(value),
                                 case["id"] + ":" + part)

    def test_duplicate_keys_rejected_before_schema(self):
        with self.assertRaises(ContractError):
            parse_message(b'{"method":"files.read","method":"files.replace"}')

    def test_nonfinite_fractional_and_surrogate_values_rejected(self):
        for raw in (b'{"x":NaN}', b'{"x":Infinity}', b'{"x":1.0}', b'{"x":"\\ud800"}'):
            with self.subTest(raw=raw), self.assertRaises(ContractError):
                parse_message(raw)

    def test_size_and_depth_limits(self):
        for raw in (b' ' * 32769, b'[' * 18 + b'0' + b']' * 18):
            with self.assertRaises(ContractError):
                parse_message(raw)

    def test_read_does_not_cross_requested_version(self):
        exchange = self.exchange("files.read")
        exchange["request"]["params"]["expected_version"] = "v_other"
        self.reject_exchange(exchange)

    def test_read_does_not_exceed_requested_bytes(self):
        exchange = self.exchange("files.read")
        exchange["request"]["params"]["length"] = 1
        self.reject_exchange(exchange)

    def test_receipt_cannot_claim_a_different_write(self):
        for field, value in (("sha256", "0" * 64), ("resource", "file_b"),
                             ("previous_version", "v_wrong"), ("version", "v1")):
            exchange = self.exchange("files.replace")
            exchange["response"]["result"]["receipt"][field] = value
            self.reject_exchange(exchange)

    def test_lost_initial_reply_can_be_queried_without_operation_id(self):
        exchange = self.exchange("operations.get")
        self.assertNotIn("operation_id", exchange["request"]["params"])
        validate_exchange(self.catalog, exchange["request"], exchange["response"])
        exchange["response"]["result"]["receipt"]["retry"]["key"] = "different_key"
        self.reject_exchange(exchange)

    def test_cancellation_response_is_correlated(self):
        exchange = self.exchange("operations.cancel")
        exchange["response"]["result"]["operation"]["operation_id"] = "wrong_operation"
        self.reject_exchange(exchange)

    def test_pagination_obeys_callers_limit(self):
        exchange = self.exchange("capabilities.list")
        exchange["request"]["params"]["limit"] = 1
        self.reject_exchange(exchange)

    def test_description_detects_catalog_drift(self):
        exchange = self.exchange("capabilities.describe")
        exchange["response"]["result"]["contract_sha256"] = "0" * 64
        self.reject_exchange(exchange)

    def test_status_does_not_add_unrequested_fields(self):
        exchange = self.exchange("system.status")
        exchange["response"]["result"]["platform"] = "R0"
        self.reject_exchange(exchange)

    def test_transport_cannot_switch_response_methods(self):
        exchange = self.exchange("files.read")
        exchange["response"]["method"] = "system.status"
        self.reject_exchange(exchange)


if __name__ == "__main__":
    unittest.main()
