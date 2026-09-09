# SPDX-License-Identifier: Apache-2.0
"""Require user-mode protection, survivor progress and complete reclamation."""
FAULTS = {
    "other-read": (14, 4, 0x700008), "other-write": (14, 6, 0x700008),
    "kernel-read": (14, 5, None), "kernel-write": (14, 7, None),
    "code-write": (14, 7, 0x400000), "stack-execute": (14, 21, 0x7ffff000),
    "stack-guard": (14, 4, 0x7fffb000), "privileged-cli": (13, 0, 0),
    "invalid-opcode": (6, 0, 0), "unsupported-fp": (7, 0, 0),
    "kernel-gate": (13, 0x40a, 0), "invalid-return": (13, 0, 0),
    "disabled-syscall": (6, 0, 0), "port-io": (13, 0, 0), "privileged-halt": (13, 0, 0),
}


def verified(serial, records):
    try:
        summaries = records(serial, "RUSTIC PROCESS ")
        if len(summaries) != 1:
            return False
        summary = {key: int(value) for key, value in summaries[0].items()}
        memory = records(serial, "RUSTIC PROCESS_MEMORY ")
        if len(memory) != 1:
            return False
        budget = {key: int(value) for key, value in memory[0].items()}
        if (budget["slots"] != 4 or budget["entry_stack_bytes"] != 20480 or budget["oom_cases"] != 3
                or not 0 < budget["metadata_bytes"] <= 8192
                or not 0 < budget["peak_frames"] <= 4 * (256 + 16)
                or budget["peak_frames"] % 4 != 0):
            return False
        expected = {"verified": 1, "ring": 3, "elf": 1, "isolated_faults": len(FAULTS),
                    "repeats": 16, "reclaimed": 1, "abi": 65536}
        if (not all(summary.get(key) == value for key, value in expected.items())
                or summary["preemptions"] < 2
                or not summary["free_before"] == summary["free_after"] > 0):
            return False
        faults = records(serial, "RUSTIC PROCESS_FAULT ")
        if len(faults) != len(FAULTS) or {f["case"] for f in faults} != set(FAULTS):
            return False
        kernel_addresses = set()
        for fault in faults:
            vector, error, address = FAULTS[fault["case"]]
            if (int(fault["vector"]) != vector or int(fault["error"], 16) != error
                    or any(fault.get(key) != value for key, value in
                           {"ring": "3", "survivor": "1", "reclaimed": "1"}.items())):
                return False
            actual = int(fault["address"], 16)
            if address is None:
                if not 0xffff800000000000 <= actual <= 0xffffffffffffffff:
                    return False
                kernel_addresses.add(actual)
            elif actual != address:
                return False
        return len(kernel_addresses) == 1
    except (KeyError, ValueError):
        return False
