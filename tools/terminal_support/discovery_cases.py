# SPDX-License-Identifier: Apache-2.0
"""Native discovery must describe the mounted volume, not a build-time promise."""
import re
from .operation_cases import references

METHODS = ("capabilities.list", "capabilities.describe", "files.read", "files.replace",
           "operations.get", "operations.cancel", "events.read", "system.status")


def capabilities(uart):
    text = uart.command("capabilities").replace("\r\n", "\n")
    found = re.findall(r"^capability-v1 method=([a-z.]+) availability=(available|degraded|unavailable)$",
                       text, re.MULTILINE)
    if [name for name, _ in found] != list(METHODS):
        raise AssertionError("discovery lost the shared catalog identity or order")
    bounds = re.findall(r"^capability-bounds-v1 max_inline_bytes=(\d+) max_page_items=(\d+) receipt_capacity=(\d+)$",
                        text, re.MULTILINE)
    if len(bounds) != 1 or any(line.startswith("error:") for line in text.splitlines()):
        raise AssertionError("missing or ambiguous discovery bounds")
    report = dict(found)
    report["bounds"] = dict(zip(("max_inline_bytes", "max_page_items", "receipt_capacity"),
                                (int(value) for value in bounds[0])))
    return report


def check(report, operations):
    """Invariants that hold on every volume this profile can mount."""
    expected = "available" if operations else "unavailable"
    if report["files.read"] != "available":
        raise AssertionError("the implemented read profile was not reported")
    if report["files.replace"] != expected or report["operations.get"] != expected:
        raise AssertionError("discovery contradicts the volume's actual operation support")
    # Only a subset of the registry contract is answered, and no digests are carried.
    if report["capabilities.list"] != "degraded" or report["capabilities.describe"] != "unavailable":
        raise AssertionError("discovery overstated the registry contract")
    for method in ("operations.cancel", "events.read", "system.status"):
        if report[method] != "unavailable":
            raise AssertionError("an unimplemented method was advertised")
    if report["bounds"] != {"max_inline_bytes": 1024, "max_page_items": 8, "receipt_capacity": 2}:
        raise AssertionError("reported bounds differ from the enforced profile")
    return report


def exercise(uart):
    # Cross-check the claim against behaviour the same shell can observe. A
    # completed-operation lookup answers Unsupported until the scoped format exists.
    workspace = references(uart, ".", "hello")[0]
    # An unused key always fails; only the reason distinguishes the formats.
    lookup = uart.command(f"operation {workspace} e_0000000000000001 k_0000000000000001", "error:")
    supported = "Unsupported" not in lookup
    report = check(capabilities(uart), supported)
    return {"verified": True, "operations_enabled": supported,
            "availability": {method: report[method] for method in METHODS},
            "bounds": report["bounds"]}


def after_reboot(uart, before):
    report = check(capabilities(uart), before["operations_enabled"])
    if {method: report[method] for method in METHODS} != before["availability"]:
        raise AssertionError("reboot changed what the service claims to implement")
