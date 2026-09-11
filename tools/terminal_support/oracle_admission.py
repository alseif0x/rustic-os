# SPDX-License-Identifier: Apache-2.0
"""Independent v4 admission decoding; CRC validation belongs to the bank reader."""
import struct


def decode(p, version, record, sequence):
    if not any(p[64:512]):
        return
    if version != 4 or any(p[81:512]):
        raise AssertionError("invalid admission extension")
    number, terminal = struct.unpack_from("<QQ", p, 64)
    state = p[80]
    if not (0 < record["previous"] < number <= sequence and 0 < record.get("instance", 0) <= number):
        raise AssertionError("invalid admission identity")
    committed = record["committed"]
    if state == 1:
        valid = terminal == committed == 0
    elif state == 2:
        valid = number < terminal <= sequence and committed == 0
    elif state == 3:
        valid = number < terminal <= sequence and committed == terminal
    else:
        valid = False
    if not valid:
        raise AssertionError("invalid admission state transition")
    record.update(admission=number, terminal=terminal, state={1: "admitted", 2: "cancelled", 3: "committed"}[state])


def numbers(record):
    return {n for n in (record["committed"], record.get("admission", 0), record.get("terminal", 0)) if n}
