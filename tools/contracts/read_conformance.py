# SPDX-License-Identifier: Apache-2.0
"""Check the complete bounded range fixture profile for files.read only.

The native SDK completes each requested bounded range. General service-v1 also
permits short non-EOF progress; this profile is stricter than shape conformance.
A bounded host fixture is not a guest service.
"""
import base64
import copy
import hashlib
import json
import re

from .read_vectors import fixture_bytes, load_cases, request_for
from .validation import ContractError, decode_data, require, validate, validate_exchange


def check_exchange(catalog, case, request, response):
    """Check one known range/challenge, including intentionally invalid requests."""
    require(isinstance(request, dict) and isinstance(response, dict), "invalid exchange")
    require(request.get("method") == response.get("method") == "files.read", "wrong read method")
    params = request.get("params")
    require(isinstance(params, dict), "missing read parameters")
    for field in ("offset", "length"):
        require(type(params.get(field)) is int and params[field] == case[field],
                "request differs from read vector")
    try:
        validate(catalog, request, "request")
    except ContractError:
        # Only these reviewed range failures may contain an invalid request.
        # Validate every remaining field by replacing solely the test range.
        require(case["expected_code"] == "invalid_request", "unexpected invalid request")
        normalized = copy.deepcopy(request)
        normalized["params"].update(offset=0, length=1)
        validate_exchange(catalog, normalized, response)
    else:
        validate_exchange(catalog, request, response)
    if case["expected_code"] is not None:
        require("error" in response and response["error"]["code"] == case["expected_code"],
                "incorrect read failure")
        require(response["error"]["effect"] == "none", "read failure invents a mutation")
        return
    require("result" in response, "read unexpectedly failed")
    result = response["result"]
    content = fixture_bytes(case["fixture"])
    expected = content[case["offset"]:case["offset"] + case["length"]]
    require(result["size"] == len(content), "incorrect fixture size")
    require(decode_data(result["data"]) == expected, "incorrect fixture bytes")
    require(result["range_sha256"] == hashlib.sha256(expected).hexdigest(), "incorrect fixture hash")


def check_exchanges(catalog, exchanges):
    """Require every shared vector exactly once and stable per-fixture identity."""
    cases = {case["id"]: case for case in load_cases()}
    require(isinstance(exchanges, list) and len(exchanges) == len(cases), "incomplete read inventory")
    seen, identities, versions = set(), {}, {}
    for exchange in exchanges:
        require(isinstance(exchange, dict) and set(exchange) == {"id", "request", "response"},
                "invalid read exchange fields")
        identity = exchange["id"]
        require(isinstance(identity, str) and identity in cases and identity not in seen,
                "unknown or repeated read vector")
        seen.add(identity)
        case, request, response = cases[identity], exchange["request"], exchange["response"]
        check_exchange(catalog, case, request, response)
        params = request["params"]
        binding = (params["workspace"], params["resource"])
        fixture = case["fixture"]
        if fixture in identities:
            require(identities[fixture] == binding, "fixture resource changed during range reads")
        else:
            require(binding not in identities.values(), "distinct fixtures alias one resource")
            identities[fixture] = binding
        if "result" in response:
            result = response["result"]
            observation = (result["version"], result["retry_epoch"])
            require(versions.setdefault(fixture, observation) == observation,
                    "fixture version or retry epoch changed during range reads")
    require(seen == set(cases), "missing read vectors")
    return len(seen)


class HostReadBackend:
    """Three immutable, explicitly scoped objects; no disk, IPC, mutation or registry."""

    workspace = "ws_read_fixture"
    version = "read_fixture_v1"
    retry_epoch = "read_fixture_epoch1"

    def __init__(self, catalog):
        self.catalog = catalog
        self.resources = {name: "file_" + name for name in ("text", "empty", "binary")}

    @staticmethod
    def failure(code):
        action = {"invalid_request": "fix_request", "access_denied": "stop",
                  "version_conflict": "refresh"}[code]
        return {"version": 1, "method": "files.read", "error": {
            "code": code, "effect": "none", "next_action": action,
        }}

    def read(self, request):
        if not isinstance(request, dict) or request.get("method") != "files.read":
            return self.failure("invalid_request")
        try:
            validate(self.catalog, request, "request")
        except ContractError:
            return self.failure("invalid_request")
        params = request["params"]
        if params["workspace"] != self.workspace or params["resource"] not in self.resources.values():
            return self.failure("access_denied")
        if params["expected_version"] not in (None, self.version):
            return self.failure("version_conflict")
        fixture = next(name for name, resource in self.resources.items() if resource == params["resource"])
        content = fixture_bytes(fixture)
        offset = params["offset"]
        if offset > len(content):
            return self.failure("invalid_request")
        data = content[offset:offset + params["length"]]
        return {"version": 1, "method": "files.read", "result": {
            "workspace": self.workspace, "resource": params["resource"], "version": self.version,
            "size": len(content), "offset": offset, "data": base64.b64encode(data).decode("ascii"),
            "range_sha256": hashlib.sha256(data).hexdigest(), "eof": offset + len(data) == len(content),
            "retry_epoch": self.retry_epoch,
        }}


def host_check(catalog, backend=None):
    backend = backend or HostReadBackend(catalog)
    exchanges = []
    for case in load_cases():
        request = request_for(case, workspace=backend.workspace, resource=backend.resources[case["fixture"]])
        exchanges.append({"id": case["id"], "request": request, "response": backend.read(request)})
    count = check_exchanges(catalog, exchanges)
    for code, changes in (("version_conflict", {"expected_version": "stale_fixture_version"}),
                          ("access_denied", {"resource": "foreign_resource"})):
        case = {"fixture": "text", "offset": 0, "length": 1024, "expected_code": code}
        request = request_for(case, workspace=backend.workspace, resource=backend.resources["text"])
        request["params"].update(changes)
        check_exchange(catalog, case, request, backend.read(request))
    return {"status": "success", "backend": "bounded_host_read_fixture", "version": 1,
            "implemented_operations": ["files.read"], "catalog_operations": len(catalog.entries),
            "fixture_profile": "complete_bounded_ranges", "range_cases": count,
            "authority_version_challenges": 2, "guest_execution": False}


def native_check(catalog, terminal):
    require(isinstance(terminal, dict) and terminal.get("verified") is True, "incomplete terminal evidence")
    require(isinstance(terminal.get("kernel_sha256"), str)
            and re.fullmatch(r"[0-9a-f]{64}", terminal["kernel_sha256"]), "missing guest image identity")
    require(type(terminal.get("boots")) is int and terminal["boots"] >= 1, "missing guest boot evidence")
    evidence = terminal.get("read_contract")
    require(isinstance(evidence, dict) and evidence.get("verified") is True, "incomplete native read evidence")
    count = check_exchanges(catalog, evidence.get("exchanges"))
    return {"status": "success", "backend": "native_uart_read_evidence", "version": 1,
            "verified_operations": ["files.read"], "catalog_operations": len(catalog.entries),
            "scope": "shared_read_range_subset", "fixture_profile": "complete_bounded_ranges",
            "range_cases": count, "guest_execution": True,
            "kernel_sha256": terminal["kernel_sha256"]}


def load_native(path):
    require(path.stat().st_size <= 1024 * 1024, "native read evidence exceeds size limit")
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "duplicate evidence JSON key")
            result[key] = value
        return result
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique)
