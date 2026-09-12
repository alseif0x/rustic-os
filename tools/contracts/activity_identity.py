# SPDX-License-Identifier: Apache-2.0
"""Validate native identities and flags before projecting them onto logical types."""
import re
from .validation import require


def native_identity(status):
    require(isinstance(status, dict), "missing native admission")
    admission = re.fullmatch(r"ad_([0-9a-f]{32})_([0-9a-f]{16})", str(status.get("id", "")))
    instance = re.fullmatch(r"si_([0-9a-f]{32})_([0-9a-f]{16})", str(status.get("instance", "")))
    require(admission is not None and instance is not None, "malformed native identity")
    lineage, number = admission[1], int(admission[2], 16)
    require(lineage != "0" * 32 and lineage == instance[1]
            and 0 < int(instance[2], 16) <= number, "native identity/instance mismatch")
    require(status.get("lineage") == lineage, "retained lineage differs from admission")
    if "number" in status:
        require(type(status["number"]) is int and status["number"] == number,
                "retained number differs from admission")
    return lineage, number


def observation_flags(observation):
    for field in ("requested", "pending"):
        require(type(observation.get(field)) is int and observation[field] in (0, 1),
                "invalid native activity flag")
    requested = observation["requested"] == 1
    require((observation["phase"] != "running" or not requested)
            and (observation["phase"] != "stopping" or requested)
            and (observation["phase"] != "queued" or observation["pending"] == 0),
            "native phase contradicts the stop flag")
    return requested
