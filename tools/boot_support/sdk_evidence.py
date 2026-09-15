# SPDX-License-Identifier: Apache-2.0
"""Require the independently compiled SDK application's guest acceptance."""
def verified(serial, records):
    try:
        values = records(serial, "RUSTIC SDK ")
        if len(values) != 1:
            return False
        value = {key: int(number) for key, number in values[0].items()}
        expected = {"verified": 1, "ring": 3, "applications": 2, "exchanges": 4,
                    "admission_rejected": 12, "parameters_rejected": 4, "reports": 4, "reclaimed": 1,
                    "heap_full": 1, "heap_reuse": 1, "heap_zeroed": 1, "heap_guarded": 1,
                    "heap_final_pages": 0}
        return (all(value.get(key) == number for key, number in expected.items())
                and value["free_before"] == value["free_after"] > 0
                # The heap actually grew, stayed inside the queried limit and
                # held bytes; the kernel decoded the guest's own summary.
                and value["heap_limit"] >= value["heap_peak_pages"] > 1
                and value["heap_peak_bytes"] > 0)
    except (KeyError, ValueError):
        return False
