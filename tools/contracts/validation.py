# SPDX-License-Identifier: Apache-2.0
"""Structural and message-local checks, never authorization or disk execution."""
import base64
import hashlib
import json

from jsonschema import ValidationError


class ContractError(ValueError):
    pass


# Suggestions apply to this invocation; a lookup denial says nothing about an
# earlier write. Services must still authorize any retry at the effect boundary.
ERROR_ACTIONS = {
    "invalid_request": {"fix_request"},
    "unsupported_version": {"refresh", "stop"},
    "access_denied": {"stop"},
    "not_found": {"refresh", "stop"},
    "version_conflict": {"refresh"},
    "idempotency_conflict": {"fix_request", "stop"},
    "expired_epoch": {"reconcile", "stop"},
    "quota_exceeded": {"retry_same", "stop"},
    "unavailable": {"retry_same", "stop"},
    "read_only": {"stop"},
    "io_error": {"retry_same", "stop"},
    "outcome_unknown": {"reconcile", "stop"},
    "cursor_expired": {"refresh"},
}


def require(condition, reason):
    if not condition:
        raise ContractError(reason)


def decode_data(value):
    try:
        data = base64.b64decode(value, validate=True)
    except (ValueError, TypeError) as error:
        raise ContractError("invalid base64") from error
    require(len(data) <= 1024, "decoded byte limit")
    require(base64.b64encode(data).decode("ascii") == value, "noncanonical base64")
    return data


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def _finite_tree(value, depth=0):
    require(depth <= 16, "nesting limit")
    require(not isinstance(value, float), "floating-point numbers are outside v1")
    if isinstance(value, str):
        value.encode("utf-8", errors="strict")
    if isinstance(value, dict):
        for key, item in value.items():
            key.encode("utf-8", errors="strict")
            _finite_tree(item, depth + 1)
    if isinstance(value, list):
        for item in value:
            _finite_tree(item, depth + 1)


def parse_message(raw):
    require(len(raw) <= 32768, "envelope byte limit")
    try:
        message = json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs)
        _finite_tree(message)
        return message
    except (UnicodeError, ValueError, RecursionError) as error:
        raise ContractError(str(error)) from error


def validate(catalog, message, direction):
    require(direction in ("request", "response"), "invalid direction")
    try:
        _finite_tree(message)
        require(len(json.dumps(message, ensure_ascii=True).encode("ascii")) <= 32768,
                "envelope byte limit")
        catalog.validator(message["method"], direction).validate(message)
    except (ValidationError, KeyError, TypeError, UnicodeError) as error:
        raise ContractError(str(error)) from error
    method = message["method"]
    value = message.get("params", message.get("result"))
    if "error" in message:
        error = message["error"]
        unknown_codes = ("expired_epoch", "outcome_unknown", "io_error", "unavailable")
        require(error["effect"] != "unknown" or error["code"] in unknown_codes,
                "error cannot describe an unknown effect")
        if error["effect"] == "unknown" or error["code"] in ("expired_epoch", "outcome_unknown"):
            require(error["effect"] == "unknown" and error["next_action"] in ("reconcile", "stop"),
                    "unknown effects must not authorize blind retry")
        else:
            require(error["next_action"] in ERROR_ACTIONS[error["code"]],
                    "incorrect recovery advice")
        if method == "operations.get":
            require(error["code"] != "not_found", "missing receipt is outcome_unknown")
        return
    if method == "files.replace" and direction == "request":
        decode_data(value["data"])
    if method == "files.read":
        if direction == "request":
            require(value["offset"] + value["length"] <= 9007199254740991,
                    "range arithmetic exceeds exact integer domain")
        else:
            data = decode_data(value["data"])
            end = value["offset"] + len(data)
            require(end <= value["size"], "range extends beyond file")
            require(value["eof"] == (end == value["size"]), "incorrect EOF")
            require(bool(data) or value["eof"], "empty non-EOF progress")
            require(hashlib.sha256(data).hexdigest() == value["range_sha256"], "range hash mismatch")
    if method == "operations.cancel" and direction == "response":
        operation = value["operation"]
        if value["disposition"] != "too_late":
            require(operation["cancel_requested"], "request flag absent")
            require(operation["state"] in ("queued", "running", "cancelled", "reconciling"),
                    "terminal operation cannot accept cancellation")


def validate_exchange(catalog, request, response):
    validate(catalog, request, "request")
    validate(catalog, response, "response")
    require(request["method"] == response["method"], "uncorrelated method")
    if "error" in response:
        return
    method, params, result = request["method"], request["params"], response["result"]
    if method == "files.read":
        for field in ("workspace", "resource", "offset"):
            require(params[field] == result[field], "read identity/range mismatch")
        require(len(decode_data(result["data"])) <= params["length"], "over-request read")
        require(params["expected_version"] in (None, result["version"]), "torn version read")
    if method == "files.replace" and result["state"] == "succeeded":
        receipt = result["receipt"]
        for field in ("workspace", "resource", "retry"):
            require(params[field] == receipt[field], "receipt identity mismatch")
        require(params["expected_version"] == receipt["previous_version"], "receipt precondition mismatch")
        data = decode_data(params["data"])
        require(receipt["size"] == len(data), "receipt size mismatch")
        require(receipt["sha256"] == hashlib.sha256(data).hexdigest(), "receipt hash mismatch")
        require(receipt["version"] != receipt["previous_version"], "version must advance")
    if method in ("capabilities.list", "events.read"):
        require(len(result["items"]) <= params["limit"], "page exceeds requested limit")
    if method == "capabilities.describe":
        described = params["method"]
        require(result["capability"]["method"] == described, "description mismatch")
        require(result["contract_id"] == catalog.entries[described][1]["$id"], "schema identity drift")
        require(result["contract_sha256"] == catalog.digest(described), "schema digest drift")
    if method == "system.status":
        require(set(result) == {"service_instance", "observation_id", *params["fields"]},
                "status must return exactly selected fields")
    if method in ("operations.get", "operations.cancel"):
        operation = result["operation"] if method.endswith("cancel") else result
        if "operation_id" in params:
            require(operation["operation_id"] == params["operation_id"], "operation mismatch")
        elif operation["state"] == "succeeded":
            for field in ("workspace", "retry"):
                require(operation["receipt"][field] == params[field], "lookup receipt mismatch")
