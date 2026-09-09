# SPDX-License-Identifier: Apache-2.0
"""Reference fixture contract shared by direct and isolated execution."""
EXPECTED = {"ok": "success", "panic": "panic", "hang": "timeout", "invalid": "fatal",
            "exception": "exception", "gp": "exception", "doublefault": "exception", "timer-stall": "timeout"}
MODES = tuple(EXPECTED)


def reached(mode, serial):
    if mode == "ok":
        return "RUSTIC IRQ verified=1 breakpoint=1 spurious=2 waits=3 cancelled=1 race_waits=100" in serial
    if mode == "hang":
        return "RUSTIC HANG deliberate=1" in serial
    if mode == "timer-stall":
        return "RUSTIC TIMER_STALL reached_wait=1" in serial
    vectors = {"exception": (6, "0x0", 0), "gp": (13, "0xfff8", 0), "doublefault": (8, "0x0", 1)}
    if mode in vectors:
        vector, error, emergency = vectors[mode]
        return ("RUSTIC FAULT_FIXTURE" in serial and
                any("RUSTIC EXCEPTION" in line and f" vector={vector} error={error} " in line
                    and line.rstrip().endswith(f"emergency={emergency}") for line in serial.splitlines()))
    return True
