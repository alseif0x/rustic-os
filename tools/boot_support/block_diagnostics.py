# SPDX-License-Identifier: Apache-2.0
"""Check failure evidence from direct kernel block fixtures, not guest success text."""
import re

PREFIX = "RUSTIC BLOCK_FAILURE "
NUMBERS = {
    "request": 64, "kind": 32, "started": 64, "now": 64,
    "elapsed_ticks": 64, "polls": 64, "expected": 16,
    "observed": 16, "device_status": 8,
}
FIELDS = set(NUMBERS) | {"phase", "reason", "descriptor", "status"}
EXPECTED = {
    "block-timeout": [(0, "Timeout", "None")],
    "block-error": [(1, "Io", "Some(1)"), (65535, "Unsupported", "Some(2)")],
}


def _records(serial):
    result = []
    for line in serial.splitlines():
        if not line.startswith(PREFIX.rstrip()):
            continue
        if not line.startswith(PREFIX):
            raise ValueError("truncated block diagnostic")
        value = {}
        for part in line[len(PREFIX):].split():
            key, separator, field = part.partition("=")
            if not separator or not field or key in value:
                raise ValueError("ambiguous block diagnostic")
            value[key] = field
        if set(value) != FIELDS:
            raise ValueError("incomplete block diagnostic")
        for key, bits in NUMBERS.items():
            text = value[key]
            if len(text) > 20 or re.fullmatch(r"0|[1-9][0-9]*", text) is None:
                raise ValueError("invalid block diagnostic integer")
            value[key] = int(text)
            if value[key] >= 1 << bits:
                raise ValueError("block diagnostic integer overflow")
        result.append(value)
    return result


def verified(mode, serial):
    try:
        failures = _records(serial)
        expected = EXPECTED.get(mode, [])
        if len(failures) != len(expected):
            return False
        index, previous_tick = 0, 0
        for value, (kind, reason, status) in zip(failures, expected):
            if (value["phase"] != "completion" or value["request"] != 0
                    or value["kind"] != kind or value["reason"] != reason
                    or value["device_status"] != 7 or value["status"] != status):
                return False
            if (value["started"] < previous_tick or value["now"] < value["started"]
                    or value["elapsed_ticks"] != value["now"] - value["started"]
                    or value["expected"] != index):
                return False
            previous_tick = value["now"]
            if reason == "Timeout":
                if (value["descriptor"] != "None" or value["observed"] != index
                        or not 1 <= value["polls"] <= 5_000_000
                        or value["elapsed_ticks"] < 25 and value["polls"] < 5_000_000):
                    return False
            else:
                # A used entry is checked before the timeout; a late actual device
                # error can therefore have elapsed >= 25 without being a Timeout.
                if (value["descriptor"] != "Some(0)"
                        or value["observed"] != (index + 1) % 65536
                        or value["polls"] >= 5_000_000):
                    return False
                index = value["observed"]
        return True
    except (KeyError, ValueError):
        return False
