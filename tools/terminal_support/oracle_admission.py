# SPDX-License-Identifier: Apache-2.0
"""Independent v4/v5 admission decoding; CRC validation belongs to the bank reader."""
import struct


def decode(p, version, record, sequence):
    if not any(p[64:512]):
        return
    if version not in (4, 5) or (version == 4 and p[81]) or any(p[82:512]):
        raise AssertionError("invalid admission extension")
    number, terminal = struct.unpack_from("<QQ", p, 64)
    state = p[80]
    reason = p[81]
    if reason > 3 or (state != 2 and reason):
        raise AssertionError("invalid prevention cause")
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
    if version == 5:
        record["prevention"] = {0: "unknown", 1: "requested", 2: "version_conflict", 3: "authority_lost"}[reason] if state == 2 else None


def numbers(record):
    return {n for n in (record["committed"], record.get("admission", 0), record.get("terminal", 0)) if n}
