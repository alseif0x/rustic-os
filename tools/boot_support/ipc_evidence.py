# SPDX-License-Identifier: Apache-2.0
"""Reject incomplete IPC acceptance or mislabeled errors."""
REJECTIONS = {
    **{name: 4 for name in ("send-null", "send-overflow", "send-kernel", "send-unmapped",
                            "send-cross-hole", "send-noncanonical", "receive-code", "receive-kernel",
                            "receive-unmapped", "receive-cross-hole", "receive-null")},
    **{name: 5 for name in ("send-empty", "send-short", "send-large", "send-huge", "receive-short")},
    "version": 6, "opcode": 7, "sender-spoof": 7,
    "foreign-handle": 2, "stale-handle": 2, "attenuation": 3,
}


def verified(serial, records):
    try:
        summaries = records(serial, "RUSTIC IPC ")
        if len(summaries) != 1:
            return False
        summary = {key: int(value) for key, value in summaries[0].items()}
        flags = ("verified", "wait", "cancel", "close", "death", "transfer", "attenuation",
                 "stale", "cross_page", "atomic_copy", "version")
        if (any(summary.get(flag) != 1 for flag in flags) or summary["ring"] != 3
                or summary["exchanges"] != 16 or summary["rejected"] != len(REJECTIONS)
                or summary["channels"] != 0 or summary["handles"] != 0
                or not summary["free_before"] == summary["free_after"] > 0):
            return False
        rejected = records(serial, "RUSTIC IPC_REJECT ")
        if len(rejected) != len(REJECTIONS) or {r["case"] for r in rejected} != set(REJECTIONS):
            return False
        return all(r["preserved"] == "1" and int(r["code"], 16) == (1 << 64) - 1 - REJECTIONS[r["case"]]
                   for r in rejected)
    except (KeyError, ValueError):
        return False
