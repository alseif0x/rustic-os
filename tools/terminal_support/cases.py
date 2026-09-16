# SPDX-License-Identifier: Apache-2.0
"""Behavior assertions run through ordinary shell commands."""
import re
import time

def pid(uart, command):
    output = uart.command(command, "started pid=")
    return int(re.search(r"started pid=(\d+)", output)[1])

def exited(uart, child, kind, code):
    deadline = time.monotonic() + 10
    while True:
        output = uart.command("ps")
        if re.search(rf"(?m)^{child} exited {kind} {code} ", output):
            return
        if time.monotonic() > deadline:
            raise AssertionError(f"process {child} failed to exit: {output}")
        time.sleep(.05)

def counters(uart):
    return {k:int(v) for k,v in re.findall(r"(free_frames|processes|channels|pending_io)=(\d+)",uart.command("mem"))}

def help_checks(uart):
    default = uart.command("help")
    assert "help advanced" in default, default
    for marker in ("pwd", "ls [PATH]", "write PATH TEXT", "cat PATH", "ps | kill PID", "tasks list PATH", "exit"):
        assert marker in default, default
    assert "run spin|fault|exit" not in default, default
    uart.command("status", "0")
    advanced_markers = (
        "select-lifecycle operations.get|operations.cancel",
        "lifecycle-profile operations.get|operations.cancel",
        "inspect-operation ADMISSION_ID",
        "enable-prevention-reasons",
        "observe-admission ADMISSION_ID",
        "retry-key PATH KEY",
        "session FILE OTHER [TICKS]",
        "stall files TICKS",
        "hold-io SKIP TICKS",
        "capabilities (implemented methods; availability is not permission)",
        "ref WORKSPACE PATH",
        "enable-operations",
        "enable-admissions",
        "act PID api-read|read-open|read-next|fill|capabilities",
    )
    assert all(marker not in default for marker in advanced_markers), default
    advanced = uart.command("help advanced")
    assert all(marker in advanced for marker in advanced_markers), advanced
    assert "run spin|fault|exit" in advanced, advanced
    uart.command("status", "0")
    for command in ("help unknown", "help advanced extra", "help extra args"):
        invalid = uart.command(command, "invalid arguments; type help")
        assert all(marker not in invalid for marker in advanced_markers), invalid
        assert "help | pwd" not in invalid, invalid
        uart.command("status", "1")
    uart.command("pwd", "/workspaces")

def transfer_measurement(uart):
    """A bounded file transfer timed from the host (#50).

    960 bytes is exactly 24 data-carrying operations of 40 bytes each in the
    current file protocol, and it fits the shell's 1024-byte line buffer. The
    timing covers the whole path the host can observe: UART, scheduling and the
    file service. It is an end-to-end figure, not a transport-only one.
    """
    payload = "x" * 960
    started = time.monotonic()
    uart.command(f'write transfer "{payload}"', "written 960 bytes")
    write_seconds = time.monotonic() - started
    started = time.monotonic()
    uart.command("cat transfer", payload)
    read_seconds = time.monotonic() - started
    # Leave the workspace as the later fixtures expect it.
    uart.command("rm transfer")
    return {"bytes": 960, "operations_per_direction": 24, "bytes_per_operation": 40,
            "write_seconds": round(write_seconds, 3), "read_seconds": round(read_seconds, 3)}


def exercise(uart):
    help_checks(uart)
    uart.command("pwd", "/workspaces")
    uart.command("ls /", "workspaces")
    uart.command('write hello "Hello from native Rust"', "written 22 bytes")
    uart.command("cat hello", "Hello from native Rust")
    uart.command("services", "mounted")
    uart.command("mem", "process_slots=8")
    uart.command("mkdir project")
    uart.command("cd project")
    uart.command("pwd", "/workspaces/project")
    uart.command("write note nested")
    uart.command("cat ../project/note", "nested")
    uart.command("cd ..")
    uart.command("rm project", "NotEmpty")
    uart.command("mkdir /system/denied", "ReadOnly")
    uart.command("cat missing", "NotFound")
    uart.command("write /system/denied value", "ReadOnly")
    uart.command("echo 'unterminated", "Quote")
    uart.command("unknown-command", "unknown command")
    uart.command("status", "1")
    uart.send(b"echo broken\x15echo repaired\r")
    assert "repaired" in uart.until()
    uart.send(b"echo cancellable\x03")
    assert "^C" in uart.until()
    uart.send(b"echo editedx\x7f\r\n")
    assert "edited\r\n" in uart.until()
    uart.send(b"write forbidden " + b"x"*1030 + b"\r")
    assert "command discarded" in uart.until()
    uart.command("cat forbidden", "NotFound")
    uart.send(b"write invalid-name \xc3\xa9\r")
    assert "ASCII command discarded" in uart.until()
    uart.command("cat invalid-name","NotFound")
    baseline = counters(uart)
    for _ in range(4):
        child = pid(uart, "run fault")
        exited(uart, child, 2, 6)
        uart.command(f"reap {child}", "exit_kind=2 code=6")
        uart.command("cat hello", "Hello from native Rust")
        child = pid(uart, "run probe hello project/note")
        exited(uart, child, 1, 0)
        uart.command(f"permissions {child}", "report=0 bytes=22 other=17")
        uart.command(f"reap {child}", "exit_kind=1 code=0")
    first = pid(uart, "run spin")
    second = pid(uart, "run spin")
    uart.command("run spin", "busy or full")
    uart.command("kill 2", "denied")
    uart.command(f"reap {first}", "busy or full")
    uart.command("cat hello", "Hello from native Rust")
    for child in (first, second):
        uart.command(f"kill {child}", "ok")
        exited(uart, child, 3, 0)
        uart.command(f"reap {child}", "exit_kind=3 code=0")
    assert counters(uart) == baseline, (baseline, counters(uart))
    child = pid(uart, "run watch hello")
    uart.command(f"revoke {child}", "ok")
    exited(uart, child, 1, 18)
    uart.command(f"permissions {child}", "rights=0")
    uart.command(f"reap {child}", "code=18")
    child = pid(uart, "run watch hello 20")
    exited(uart, child, 1, 19)
    uart.command(f"reap {child}", "code=19")
    for _ in range(3):
        child = pid(uart, "run watch hello")
        uart.command("restart files", "utility sessions revoked")
        uart.command("cat hello", "Hello from native Rust")
        uart.command(f"kill {child}", "denied")
        assert counters(uart) == baseline, (baseline, counters(uart))
    uart.command("rm project/note")
    uart.command("rm project")
    uart.command("cat /config/owner-policy", "helpers=explicit")
    uart.command("write /config/owner-policy invalid")
    uart.command("restart files", "utility sessions revoked")
    uart.command("run read hello", "denied")
    uart.command("cat hello", "Hello from native Rust")
    uart.command(r'write /config/owner-policy "rustic-owner-v1\nhelpers=explicit\n"')
    uart.command("restart files", "utility sessions revoked")
    child = pid(uart, "run read hello")
    exited(uart, child, 1, 0)
    uart.command(f"reap {child}", "code=0")
    return uart.commands
