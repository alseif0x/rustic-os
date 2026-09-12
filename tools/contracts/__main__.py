# SPDX-License-Identifier: Apache-2.0
"""Validate canonical contracts, check the read subset or export descriptors."""
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
    parser.add_argument("command", choices=("check", "export", "read-check", "read-native", "operations-check", "operations-native", "activity-check", "activity-native", "capabilities-check", "capabilities-native", "lifecycle-check", "lifecycle-native", "lifecycle-export", "negotiation-native"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--evidence", type=Path, help="terminal.json from the native read fixture")
    args = parser.parse_args()
    if (args.command in ("read-native", "operations-native", "activity-native", "capabilities-native", "lifecycle-native", "negotiation-native")) != (args.evidence is not None):
        parser.error("--evidence is required only for native evidence commands")
    catalog = Catalog()
    if args.command == 'negotiation-native':
        from .negotiation_conformance import native_check
        from .read_conformance import load_native
        result = native_check(load_native(args.evidence))
    elif args.command.startswith('lifecycle-'):
        from .lifecycle_conformance import catalog as lifecycle_catalog, host_check, native_check
        from .read_conformance import load_native
        catalog = lifecycle_catalog()
        result = (catalog.descriptors() if args.command == 'lifecycle-export' else host_check(catalog) if args.command == 'lifecycle-check' else native_check(catalog, load_native(args.evidence)))
    elif args.command == "check":
        result = check(catalog)
    elif args.command == "export":
        result = catalog.descriptors()
    elif args.command in ("capabilities-check", "capabilities-native"):
        from .capabilities_conformance import host_check, native_check
        from .read_conformance import load_native
        result = host_check(catalog) if args.command == "capabilities-check" else native_check(catalog, load_native(args.evidence))
    elif args.command in ("activity-check", "activity-native"):
        from .activity_conformance import host_check, native_check
        from .read_conformance import load_native
        result = host_check(catalog) if args.command == "activity-check" else native_check(catalog, load_native(args.evidence))
    elif args.command in ("operations-check", "operations-native"):
        from .operation_conformance import host_check, native_check
        from .read_conformance import load_native
        result = host_check(catalog) if args.command == "operations-check" else native_check(catalog, load_native(args.evidence))
    else:
        from .read_conformance import host_check, load_native, native_check
        result = (host_check(catalog) if args.command == "read-check"
                  else native_check(catalog, load_native(args.evidence)))
    content = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(content)
    else:
        print(content, end="")


if __name__ == "__main__":
    main()
