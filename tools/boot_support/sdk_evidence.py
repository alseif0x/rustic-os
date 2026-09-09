# SPDX-License-Identifier: Apache-2.0
"""Require the independently compiled SDK application's guest acceptance."""
def verified(serial, records):
    try:
        values = records(serial, "RUSTIC SDK ")
        if len(values) != 1:
            return False
        value = {key: int(number) for key, number in values[0].items()}
        expected = {"verified": 1, "ring": 3, "applications": 2, "exchanges": 4,
                    "admission_rejected": 12, "parameters_rejected": 4, "reports": 2, "reclaimed": 1}
        return (all(value.get(key) == number for key, number in expected.items())
                and value["free_before"] == value["free_after"] > 0)
    except (KeyError, ValueError):
        return False
