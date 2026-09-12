# SPDX-License-Identifier: Apache-2.0
"""Reference fixture contract shared by direct and isolated execution."""
from .process_evidence import verified as process_verified
from .ipc_evidence import verified as ipc_verified
from .sdk_evidence import verified as sdk_verified
from .block_evidence import MODES as BLOCK_MODES, verified as block_verified
MEMORY_FAULTS = {"memory-ro": (3, "MemoryReadOnly"), "memory-nx": (17, "MemoryNx"),
                 "memory-unmapped": (0, "MemoryUnmapped"), "memory-text-alias": (3, "MemoryTextAlias"),
                 "memory-guard": (0, "MemoryGuard")}
EXPECTED = {"ok": "success", "panic": "panic", "hang": "timeout", "invalid": "fatal",
            "exception": "exception", "gp": "exception", "doublefault": "exception", "timer-stall": "timeout"}
EXPECTED.update({mode: "exception" for mode in MEMORY_FAULTS})
EXPECTED.update({mode: "success" for mode in BLOCK_MODES})
EXPECTED["terminal-test"]="success"
EXPECTED["recovery-test"]="success"
MODES = tuple(EXPECTED) + ("terminal", "terminal-init")


def reached(mode, serial):
    if mode == "recovery-test":
        return (serial.count("RusticOS native terminal 0.1") == 98
                and serial.count("error: Uncertain") == 17
                and "operation-v1" in serial
                and "persistent format v3" in serial
                and "persistent format v4" in serial
                and "admission-v1" in serial
                and serial.count("RUSTIC IO_OBSERVATION held=1") == 31
                and serial.count("admission-activity-v1") == 91
                and serial.count("admission-observation-v1") == 13
                and "phase=queued" in serial
                and "IdempotencyConflict" in serial
                and "ExpiredEpoch" in serial
                and "RUSTIC PANIC" not in serial)
    if mode == "terminal-test":
        return serial.count("RUSTIC TERMINAL stopped=1 reclaimed=1") == 2 and serial.count("RusticOS native terminal 0.1") == 2 and "RUSTIC PANIC" not in serial
    if mode in BLOCK_MODES:
        return block_verified(mode, serial, records)
    if mode == "ok":
        return ("RUSTIC IRQ verified=1 breakpoint=1 spurious=2 waits=3 cancelled=1 race_waits=100" in serial
                and memory_verified(serial) and process_verified(serial, records) and ipc_verified(serial, records)
                and sdk_verified(serial, records))
    if mode == "hang":
        return "RUSTIC HANG deliberate=1" in serial
    if mode == "timer-stall":
        return "RUSTIC TIMER_STALL reached_wait=1" in serial
    if mode in MEMORY_FAULTS:
        return memory_fault(mode, serial)
    vectors = {"exception": (6, "0x0", 0), "gp": (13, "0xfff8", 0), "doublefault": (8, "0x0", 1)}
    if mode in vectors:
        vector, error, emergency = vectors[mode]
        return ("RUSTIC FAULT_FIXTURE" in serial and
                any("RUSTIC EXCEPTION" in line and f" vector={vector} error={error} " in line
                    and line.rstrip().endswith(f"emergency={emergency}") for line in serial.splitlines()))
    return True


def records(serial, prefix):
    return [dict(part.split("=", 1) for part in line[len(prefix):].split() if "=" in part)
            for line in serial.splitlines() if line.startswith(prefix)]


def memory_verified(serial):
    try:
        for raw in records(serial, "RUSTIC MEMORY "):
            values = {key: int(value) for key, value in raw.items()}
            expected = {"verified": 1, "page_bytes": 4096, "metadata_bytes": 65536,
                        "rollback": 1, "zero_reuse": 1, "spaces": 2, "wx": 1, "aliases": 1, "guard": 1}
            if (all(values.get(key) == value for key, value in expected.items())
                    and values["free_before"] == values["free_after"] == values["exhausted"] > 0
                    and values["managed_frames"] - values["free_before"] == values["table_frames"] > 0):
                return True
    except (KeyError, ValueError):
        pass
    return False


def memory_fault(mode, serial):
    expected_error, fixture = MEMORY_FAULTS[mode]
    try:
        for start in records(serial, "RUSTIC MEMORY_FAULT "):
            if start.get("mode") != fixture:
                continue
            for fault in records(serial, "RUSTIC EXCEPTION "):
                if (int(fault["vector"]) == 14 and int(fault["error"], 16) == expected_error
                        and int(fault["cr2"], 16) == int(start["address"], 16)
                        and fault["emergency"] == "0"):
                    return True
    except (KeyError, ValueError):
        pass
    return False
