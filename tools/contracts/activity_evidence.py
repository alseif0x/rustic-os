# SPDX-License-Identifier: Apache-2.0
"""Validate the native live-control mission without claiming service-v1 mapping."""
import re
from .validation import require


def check_activity(cases):
    expected = {
        "early": (0, 8, 0, False),
        "header": (15, 8, 0, True),
        "flush": (16, 8, 0, True),
        "inspect_only": (0, 4, 17, True),
        "foreign_scope": (0, 8, 27, True),
    }
    for name in (*expected, "failed_drain", "saturated"):
        found = [case for case in cases if case.get("case") == "public_activity_" + name]
        require(len(found) == 1, "missing or duplicate native activity case")
        case = found[0]
        require(case.get("reboot_verified") is True, "activity outcome not checked after reboot")
        require(isinstance(case.get("sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", case["sha256"]),
                "missing independent activity disk identity")
        if name == "failed_drain":
            require(case.get("uncertain") is True and "committed" not in case,
                    "failed drain fabricated a definitive result")
            continue
        if name == "saturated":
            # A saturated run still has to show owner progress and prevention.
            require(all(case.get(field) is True for field in
                        ("staging_full", "undrained_client", "owner_progress", "stopped"))
                    and case.get("committed") is False,
                    "saturated execution lacks owner progress or durable prevention")
            continue
        skip, rights, denied, committed = expected[name]
        require(all(type(case.get(field)) is int and case[field] == value
                    for field, value in (("skip", skip), ("rights", rights), ("denied", denied)))
                and case.get("committed") is committed and case.get("status_during_io") is True,
                "missing or contradictory public activity evidence")
