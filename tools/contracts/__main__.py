# SPDX-License-Identifier: Apache-2.0
"""Validate fixture shapes or export descriptors from the canonical catalog."""
import argparse
import json
from pathlib import Path

from .catalog import Catalog
from .validation import ContractError, validate, validate_exchange


def check(catalog):
    cases = json.loads((catalog.root / "fixtures/messages.json").read_text())
    identifiers = set()
    for case in cases:
        if case["id"] in identifiers:
            raise ValueError("duplicate fixture ID")
        identifiers.add(case["id"])
        try:
            validate(catalog, case["message"], case["direction"])
            valid = True
        except ContractError:
            valid = False
        if valid != case["valid"]:
            raise ValueError("incorrect acceptance: " + case["id"])
    exchanges = json.loads((catalog.root / "fixtures/exchanges.json").read_text())
    for exchange in exchanges:
        validate_exchange(catalog, exchange["request"], exchange["response"])
    return {"backend": "schema_and_message_checks_only", "version": 1,
            "operations": len(catalog.entries), "message_fixtures": len(cases),
            "exchange_fixtures": len(exchanges), "guest_execution": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "export"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    catalog = Catalog()
    result = check(catalog) if args.command == "check" else catalog.descriptors()
    content = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(content)
    else:
        print(content, end="")


if __name__ == "__main__":
    main()
