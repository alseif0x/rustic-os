# SPDX-License-Identifier: Apache-2.0
"""Fixed native owner workload. Timing wraps verified UART effects, not sleeps."""
import re
import time
from terminal_support.cases import counters, pid
from terminal_support.authority_cases import actor, cleanup, fence
from terminal_support.management_cases import start_restart, wait_job


def timed(metrics, name, action):
    started = time.perf_counter()
    result = action()
    metrics[name] = time.perf_counter() - started
    return result


def exercise(uart, metrics, injected_ticks):
    resident = counters(uart)
    assert resident["processes"] == 3 and resident["channels"] == 4 and resident["pending_io"] == 0
    uart.command("write a seed")
    uart.command("write b untouched")
    if injected_ticks:
        uart.command(f"stall files {injected_ticks}", "diagnostic armed")
    timed(metrics, "read_seconds", lambda: uart.command("cat a", "seed"))
    # Both control and injected workloads restart here; injection changes only
    # the service's actual wait before the measured read.
    uart.command("restart files", "utility sessions revoked")
    assert counters(uart) == resident

    c = pid(uart, "session a b")
    h = pid(uart, f"helper {c} a b")
    for child in (c, h):
        result = actor(uart, child, "read")
        assert result["other"] == 17 and result["control_denied"] == 1
    actor(uart, c, "stage")
    for child in (c, h):
        fill = actor(uart, child, "flood")
        assert fill["value"] >= 4 and fill["other"] > 0
    occupied = counters(uart)
    assert occupied["processes"] == 5 and occupied["channels"] == 8 and occupied["pending_io"] == 0
    uart.command("run spin", "busy or full")
    metrics["sampled_pressure_extra_frames"] = resident["free_frames"] - occupied["free_frames"]
    timed(metrics, "pressure_control_seconds", lambda: uart.command("mem", "processes=5 channels=8"))
    timed(metrics, "pressure_write_seconds", lambda: uart.command("write a owner", "written 5 bytes"))
    timed(metrics, "revoke_seconds", lambda: fence(uart, h, "access=fenced members=2 discarded_staging=1 effects=settled"))
    for child in (c, h):
        assert actor(uart, child, "drain")["other"] >= 1
        actor(uart, child, "read", 18)
    cleanup(uart, c, h)
    assert counters(uart) == resident

    # Four roots, owner policy and A/B leave exactly 25 object slots.
    for index in range(25):
        uart.command(f"touch q{index}")
    uart.command("touch overflow", "Full")
    timed(metrics, "full_control_seconds", lambda: uart.command("mem", "pending_io=0"))
    for index in range(25):
        uart.command(f"rm q{index}")
    assert counters(uart) == resident

    uart.command("stall files 0", "diagnostic armed")
    timed(metrics, "stopped_control_seconds", lambda: uart.command("mem", "pending_io=0"))
    uart.send(b"cat a\r")
    uart.until(b"cat a\r\n")

    def interrupt():
        uart.send(b"\x03")
        result = uart.until()
        assert "error: Interrupted" in result, result

    timed(metrics, "interrupt_seconds", interrupt)
    wait_job(uart, start_restart(uart))
    assert counters(uart) == resident

    uart.command("hold-io 0 200", "diagnostic armed")
    uart.send(b"write a pending\r")
    uart.until(b"RUSTIC IO_OBSERVATION held=1")
    uart.send(b"\x03")
    assert "error: Uncertain" in uart.until()
    timed(metrics, "admitted_control_seconds", lambda: uart.command("mem", "pending_io=1"))
    job = start_restart(uart)
    pending = uart.command(f"job-status {job}", "pending_io=1")
    assert "phase=2" in pending
    timed(metrics, "drain_seconds", lambda: wait_job(uart, job))
    assert counters(uart) == resident
    uart.command("cat a", "owner")
    uart.command("cat b", "untouched")
    return resident


def shutdown(uart, process, resident, metrics):
    uart.send(b"exit\r")
    uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
    # The marker precedes its numeric suffix, so collect the rest from the
    # transcript after the guest exits, without assuming socket chunk boundaries.
    if process.wait(timeout=10) != 33:
        raise AssertionError("unclean measurement shutdown")
    uart.socket.settimeout(1)
    while chunk := uart.socket.recv(4096):
        uart.data.extend(chunk)
    uart.save()
    serial = uart.data.decode("ascii", "backslashreplace")
    values = re.findall(r"RUSTIC TERMINAL stopped=1 reclaimed=1 free_before=(\d+) free_after=(\d+)", serial)
    assert len(values) == 1 and values[0][0] == values[0][1], serial[-1000:]
    metrics["resident_runtime_frames"] = int(values[0][0]) - resident["free_frames"]
    assert metrics["resident_runtime_frames"] > 0
