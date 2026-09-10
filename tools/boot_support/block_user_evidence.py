# SPDX-License-Identifier: Apache-2.0
"""Require ring-3 storage, owned cleanup and control progress; markers alone cannot pass."""
def verified(mode, serial, records):
    try:
        values = records(serial, "RUSTIC BLOCK_USER ")
        phases = ["write", "read"] if mode == "block-user" else ["faults"]
        if [value["phase"] for value in values] != phases:
            return False
        for value in values:
            faults = value["phase"] == "faults"
            expected = {"verified": 1, "ring": 3, "applications": 14 if faults else 1,
                        "rejected": 53 if faults else 0, "lifecycle": 6 if faults else 0,
                        "max_bytes": 512, "queue_slots": 2, "handle_slots": 4, "dma_frames": 3}
            if any(int(value.get(key, -1)) != number for key, number in expected.items()):
                return False
            if not int(value["free_before"]) == int(value["free_after"]) > 0:
                return False
            if int(value["control_preemptions"]) <= 0 or int(value["peak_frames"]) <= 3:
                return False
        return True
    except (KeyError, ValueError):
        return False
