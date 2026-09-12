# SPDX-License-Identifier: Apache-2.0
"""Native discovery checked against the reviewed capability type.

The guest reports which catalog methods it implements. This validates those
claims as `capability` items and cross-checks them against the rest of the same
native evidence, so a service cannot advertise a method the run never used. It
is not an implementation of `capabilities.list`: the guest carries no service
instance for discovery and no contract digest, so the full method output and
`capabilities.describe` remain unimplemented.
"""
from jsonschema import Draft202012Validator
from .validation import require

METHODS = ("capabilities.list", "capabilities.describe", "files.read", "files.replace",
           "operations.get", "operations.cancel", "events.read", "system.status")
AVAILABILITY = ("available", "degraded", "unavailable")
BOUNDS = {"max_inline_bytes": 1024, "max_page_items": len(METHODS), "receipt_capacity": 2}


def validator(catalog):
    return Draft202012Validator(
        {"$ref": "urn:rusticos:services:v1:types#/$defs/capability"},
        registry=catalog.registry,
    )


def items(catalog, availability):
    """Each reported method as one reviewed `capability`; order is the catalog's."""
    require(list(availability) == list(METHODS), "discovery lost the catalog identity or order")
    checker = validator(catalog)
    result = []
    for method in METHODS:
        item = {"method": method, "version": 1, "availability": availability[method]}
        require(not list(checker.iter_errors(item)), "invalid capability item")
        result.append(item)
    return result


def check_discovery(catalog, discovery, exercised):
    require(isinstance(discovery, dict) and discovery.get("verified") is True,
            "missing native discovery evidence")
    reported = discovery.get("availability")
    require(isinstance(reported, dict), "missing reported availability")
    entries = items(catalog, reported)
    require(discovery.get("bounds") == BOUNDS, "reported bounds differ from the profile")
    operations = discovery.get("operations_enabled")
    require(isinstance(operations, bool), "discovery did not record the volume's support")
    expected = "available" if operations else "unavailable"
    require(reported["files.replace"] == expected and reported["operations.get"] == expected,
            "discovery contradicts the volume's actual operation support")
    # A method may only be advertised as implemented when this same run used it.
    for method, used in exercised.items():
        if not used:
            require(reported[method] == "unavailable",
                    "an unexercised method was advertised as implemented")
    require(reported["capabilities.list"] == "degraded"
            and reported["capabilities.describe"] == "unavailable",
            "discovery overstated the registry contract")
    return entries


def native_check(catalog, evidence):
    require(isinstance(evidence, dict) and evidence.get("verified") is True, "unverified evidence")
    require(isinstance(evidence.get("kernel_sha256"), str)
            and len(evidence["kernel_sha256"]) == 64, "missing guest identity")
    discovery = evidence.get("discovery")
    exercised = {
        "files.read": bool(evidence.get("read_contract")),
        "operations.cancel": False,
        "events.read": False,
        "system.status": False,
    }
    entries = check_discovery(catalog, discovery, exercised)
    return {"status": "success", "backend": "native_uart_discovery_evidence",
            "mapped_types": ["capability"], "unimplemented_methods":
            ["capabilities.list", "capabilities.describe"],
            "profile": "owned_subset_discovery", "items": len(entries),
            "availability": {entry["method"]: entry["availability"] for entry in entries},
            "guest_execution": True, "kernel_sha256": evidence["kernel_sha256"]}


def host_check(catalog):
    """Enumerate the declared shape, including the vectors it must reject."""
    available = dict.fromkeys(METHODS, "unavailable")
    available.update({"capabilities.list": "degraded", "files.read": "available"})
    accepted = len(items(catalog, available))
    renamed = {("files.write" if method == "system.status" else method): value
               for method, value in available.items()}
    broken = ({**available, "files.read": "implemented"},
              {**available, "files.read": None},
              {method: available[method] for method in METHODS[:-1]},
              renamed)
    rejected = 0
    for report in broken:
        try:
            items(catalog, report)
        except Exception:
            rejected += 1
    require(rejected == len(broken), "the declared shape accepted an invalid report")
    return {"status": "success", "backend": "declared_capability_items",
            "mapped_types": ["capability"], "unimplemented_methods":
            ["capabilities.list", "capabilities.describe"],
            "profile": "owned_subset_discovery", "accepted_items": accepted,
            "rejected_vectors": rejected, "guest_execution": False}
