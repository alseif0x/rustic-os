# SPDX-License-Identifier: Apache-2.0
import copy
import base64
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.contracts.catalog import Catalog
from tools.contracts.read_conformance import HostReadBackend, host_check, load_native, native_check
from tools.contracts.read_vectors import fixture_bytes, load_cases, request_for
from tools.contracts.validation import ContractError, validate_exchange


class ReadConformance(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.catalog = Catalog()

    def evidence(self):
        backend = HostReadBackend(self.catalog)
        exchanges = []
        for case in load_cases():
            request = request_for(case, workspace=backend.workspace,
                                  resource=backend.resources[case["fixture"]])
            exchanges.append({"id": case["id"], "request": request, "response": backend.read(request)})
        # Synthetic shape used to test the evidence checker, not actual guest execution.
        return {"verified": True, "boots": 2, "kernel_sha256": "0" * 64,
                "read_contract": {"verified": True, "exchanges": exchanges}}

    def test_host_subset_passes_all_shared_ranges_and_challenges(self):
        result = host_check(self.catalog)
        self.assertEqual(result["range_cases"], 15)
        self.assertEqual(result["authority_version_challenges"], 2)
        self.assertEqual(result["implemented_operations"], ["files.read"])
        self.assertFalse(result["guest_execution"])

    def test_boundary_vectors_are_not_accidentally_changed(self):
        self.assertEqual(fixture_bytes("text"), b"Hello from native Rust")
        self.assertEqual(fixture_bytes("empty"), b"")
        binary = fixture_bytes("binary")
        self.assertEqual(len(binary), 1024)
        self.assertEqual(binary[:4], bytes((11, 48, 85, 122)))
        cases = {case["id"]: case for case in load_cases()}
        self.assertEqual({cases["padding_" + str(n)]["length"] for n in (55, 56, 63, 64, 65)},
                         {55, 56, 63, 64, 65})

    def test_bad_hash_or_eof_backend_fails_conformance(self):
        for field, replacement in (("range_sha256", "0" * 64), ("eof", False)):
            backend = HostReadBackend(self.catalog)
            read = backend.read
            def broken(request):
                response = read(request)
                if "result" in response:
                    response["result"][field] = replacement
                return response
            backend.read = broken
            with self.subTest(field=field), self.assertRaises(ContractError):
                host_check(self.catalog, backend)

    def test_backend_ignoring_requested_version_is_detected(self):
        backend = HostReadBackend(self.catalog)
        read = backend.read
        def broken(request):
            altered = copy.deepcopy(request)
            altered["params"]["expected_version"] = None
            return read(altered)
        backend.read = broken
        with self.assertRaises(ContractError):
            host_check(self.catalog, backend)

    def test_backend_accepting_foreign_resource_is_detected(self):
        backend = HostReadBackend(self.catalog)
        read = backend.read
        def broken(request):
            altered = copy.deepcopy(request)
            foreign = altered["params"]["resource"] == "foreign_resource"
            if foreign:
                altered["params"]["resource"] = backend.resources["text"]
            response = read(altered)
            if foreign and "result" in response:
                response["result"]["resource"] = "foreign_resource"
            return response
        backend.read = broken
        with self.assertRaises(ContractError):
            host_check(self.catalog, backend)

    def test_partial_backend_does_not_accept_other_operations(self):
        backend = HostReadBackend(self.catalog)
        response = backend.read({"version": 1, "method": "system.status", "params": {"fields": []}})
        self.assertEqual(response["error"]["code"], "invalid_request")

    def test_complete_evidence_shape_passes_range_checker(self):
        result = native_check(self.catalog, self.evidence())
        self.assertEqual(result["range_cases"], 15)
        self.assertEqual(result["scope"], "shared_read_range_subset")

    def test_short_read_is_valid_v1_but_outside_complete_range_profile(self):
        evidence = self.evidence()
        exchange = evidence["read_contract"]["exchanges"][2]
        short = fixture_bytes("binary")[:32]
        exchange["response"]["result"].update(
            data=base64.b64encode(short).decode("ascii"),
            range_sha256=hashlib.sha256(short).hexdigest())
        validate_exchange(self.catalog, exchange["request"], exchange["response"])
        with self.assertRaises(ContractError):
            native_check(self.catalog, evidence)

    def test_missing_duplicate_or_unknown_case_is_rejected(self):
        for mutation in ("missing", "duplicate", "unknown"):
            evidence = self.evidence()
            exchanges = evidence["read_contract"]["exchanges"]
            if mutation == "missing":
                exchanges.pop()
            elif mutation == "duplicate":
                exchanges[-1] = copy.deepcopy(exchanges[0])
            else:
                exchanges[-1]["id"] = "invented_case"
            with self.subTest(mutation=mutation), self.assertRaises(ContractError):
                native_check(self.catalog, evidence)

    def test_read_or_terminal_failure_and_missing_identity_are_rejected(self):
        for field in ("terminal", "read", "hash", "boots"):
            evidence = self.evidence()
            if field == "terminal":
                evidence["verified"] = False
            elif field == "read":
                evidence["read_contract"]["verified"] = 1
            elif field == "hash":
                evidence.pop("kernel_sha256")
            else:
                evidence["boots"] = True
            with self.subTest(field=field), self.assertRaises(ContractError):
                native_check(self.catalog, evidence)

    def test_consistent_but_wrong_data_and_hash_are_rejected(self):
        evidence = self.evidence()
        result = evidence["read_contract"]["exchanges"][0]["response"]["result"]
        result.update(data="WA==", size=1, range_sha256=hashlib.sha256(b"X").hexdigest())
        with self.assertRaises(ContractError):
            native_check(self.catalog, evidence)

    def test_fixture_binding_or_version_cannot_change_between_ranges(self):
        for field in ("resource", "version", "retry_epoch"):
            evidence = self.evidence()
            exchange = evidence["read_contract"]["exchanges"][3]
            exchange["response"]["result"][field] = "other_identity"
            if field == "resource":
                exchange["request"]["params"][field] = "other_identity"
            with self.subTest(field=field), self.assertRaises(ContractError):
                native_check(self.catalog, evidence)

    def test_invalid_ranges_do_not_hide_unrelated_invalid_fields(self):
        for field, value in (("resource", "bad/path"), ("expected_version", True)):
            evidence = self.evidence()
            evidence["read_contract"]["exchanges"][-1]["request"]["params"][field] = value
            with self.subTest(field=field), self.assertRaises(ContractError):
                native_check(self.catalog, evidence)

    def test_vector_range_and_failure_code_are_bound_to_evidence(self):
        for mutation in ("range", "code"):
            evidence = self.evidence()
            exchange = evidence["read_contract"]["exchanges"][-1]
            if mutation == "range":
                exchange["request"]["params"]["length"] = 2
            else:
                exchange["response"]["error"].update(code="access_denied", next_action="stop")
            with self.subTest(mutation=mutation), self.assertRaises(ContractError):
                native_check(self.catalog, evidence)

    def test_duplicate_evidence_keys_and_oversized_evidence_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "terminal.json"
            for content in ('{"verified":false,"verified":true}', " " * (1024 * 1024 + 1)):
                path.write_text(content)
                with self.assertRaises(ContractError):
                    load_native(path)
            path.write_text(json.dumps(self.evidence()))
            self.assertEqual(load_native(path)["boots"], 2)


if __name__ == "__main__":
    unittest.main()
