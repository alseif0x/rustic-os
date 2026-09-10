# SPDX-License-Identifier: Apache-2.0
"""Shared bounded read inputs; standard library only, safe for the UART harness."""
import json
from pathlib import Path


PATH = Path(__file__).resolve().parents[2] / "contracts/services/v1/fixtures/read-ranges.json"
MAX_INTEGER = (1 << 53) - 1


def _document():
    if PATH.stat().st_size > 16384:
        raise ValueError("read vectors exceed their size bound")
    document = json.loads(PATH.read_text(encoding="utf-8"))
    if document["version"] != 1:
        raise ValueError("unsupported read vector version")
    return document


def fixture_bytes(name):
    fixture = _document()["fixtures"][name]
    if fixture["encoding"] == "utf8":
        content = fixture["value"].encode("utf-8")
    elif fixture["encoding"] == "linear_u8":
        length = fixture["length"]
        if type(length) is not int or not 0 <= length <= 1024:
            raise ValueError("read fixture length exceeds inline limit")
        content = bytes((i * fixture["multiplier"] + fixture["increment"]) % 256
                        for i in range(length))
    else:
        raise ValueError("unsupported read fixture encoding")
    if len(content) > 1024:
        raise ValueError("read fixture exceeds inline limit")
    return content


def load_cases():
    document = _document()
    cases = document["cases"]
    identifiers = set()
    if not isinstance(cases, list) or not 1 <= len(cases) <= 64:
        raise ValueError("invalid read case inventory")
    for case in cases:
        if set(case) != {"id", "fixture", "offset", "length", "expected_code"}:
            raise ValueError("invalid read case fields")
        if not isinstance(case["id"], str) or not case["id"] or case["id"] in identifiers:
            raise ValueError("duplicate or invalid read case identity")
        identifiers.add(case["id"])
        if case["fixture"] not in document["fixtures"]:
            raise ValueError("unknown read fixture")
        if any(type(case[key]) is not int or not 0 <= case[key] <= MAX_INTEGER
               for key in ("offset", "length")):
            raise ValueError("invalid read range vector")
        if case["expected_code"] not in (None, "invalid_request"):
            raise ValueError("unsupported expected range result")
    return cases


def request_for(case, *, workspace, resource, expected_version=None):
    return {"version": 1, "method": "files.read", "params": {
        "workspace": workspace, "resource": resource, "expected_version": expected_version,
        "offset": case["offset"], "length": case["length"],
    }}
