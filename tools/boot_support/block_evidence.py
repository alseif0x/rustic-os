# SPDX-License-Identifier: Apache-2.0
"""Block-device acceptance on the fixed 4 GiB test disk."""
MODES = ("block-persist", "block-readonly", "block-error", "block-timeout", "block-missing", "block-user", "block-user-faults")
PHASES = {"block-persist": ["write", "read"], "block-readonly": ["readonly"],
          "block-error": ["error"], "block-timeout": ["timeout"], "block-missing": ["missing"]}


def verified(mode, serial, records):
    if mode in ("block-user", "block-user-faults"):
        from .block_user_evidence import verified as user_verified
        return user_verified(mode, serial, records)
    try:
        values = records(serial, "RUSTIC BLOCK ")
        if [value["phase"] for value in values] != PHASES[mode]:
            return False
        for value in values:
            missing = value["phase"] == "missing"
            expected = {"verified": 1, "sectors": 8388608, "sector_bytes": 512, "max_bytes": 512,
                        "dma_frames": 0 if missing else 3, "rejected": 0 if missing else 5}
            if any(int(value.get(key, -1)) != number for key, number in expected.items()):
                return False
            if not int(value["free_before"]) == int(value["free_after"]) > 0:
                return False
        return True
    except (KeyError, ValueError):
        return False
