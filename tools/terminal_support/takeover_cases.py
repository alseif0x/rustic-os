# SPDX-License-Identifier: Apache-2.0
"""Real stopped service, live owner UART, late acknowledgments and explicit recovery."""
import time
from .cases import pid, counters, exited
from .authority_cases import actor, actor_result, fence, cleanup
from .oracle import snapshot

def exercise(uart, data):
    baseline = counters(uart)
    uart.command("write takeover-a original")
    uart.command("write takeover-b untouched")
    c = pid(uart,"session takeover-a takeover-b")
    h = pid(uart,f"helper {c} takeover-a takeover-b")
    actor(uart,c,"read")
    actor(uart,c,"stage")
    # The service acknowledges this diagnostic before ceasing to read any channel.
    uart.command("stall files 600", "stall diagnostic armed")
    durations = []
    def control(command, expected=None):
        started = time.monotonic()
        value = uart.command(command,expected)
        durations.append(time.monotonic()-started)
        # Regression tripwire for the former 10-second nested waits, not a product SLO.
        assert durations[-1] < 2, (command,durations[-1])
        return value
    control(f"act {c} commit", "actor state=pending")
    control(f"revoke {h}", "access=requested")
    control(f"revocation {c}", "effects=unknown")
    control("services", "control-pending")
    control("mem", "pending_io=0")
    control("pwd", "/workspaces")
    control("echo owner-still-responsive", "owner-still-responsive")
    control(f"permissions {h}", "rights=0")
    control(f"helper {c} takeover-a takeover-b", "denied")
    deadline = time.monotonic() + 12
    while "access=unconfirmed" not in control(f"revocation {c}"):
        assert time.monotonic() < deadline, "missing guest observation deadline"
        time.sleep(.05)
    control(f"actor-status {c}", "state=unconfirmed")
    # The same admitted request eventually receives its own late reply; no resend.
    result = fence(uart,c,"discarded_staging=1 effects=settled",request=False)
    actor_result(uart,c,18)
    uart.command("cat takeover-a","original")
    actor(uart,h,"read",18)
    cleanup(uart,c,h)
    assert counters(uart) == baseline

    # Indefinite stall: pending actor and revocation cannot block owner kill/reap,
    # non-file process launch, or restart. Their old contexts never resume.
    c = pid(uart,"session takeover-a takeover-b")
    h = pid(uart,f"helper {c} takeover-a takeover-b")
    actor(uart,c,"read"); actor(uart,c,"stage")
    uart.command("stall files 0","stall diagnostic armed")
    control(f"act {c} commit","actor state=pending")
    control(f"kill {c}","ok")
    exited(uart,c,3,0)
    control(f"reap {c}","code=0")
    pending = control(f"revocation {c}","effects=unknown")
    assert "access=requested" in pending or "access=unconfirmed" in pending, pending
    control(f"permissions {h}","rights=0")
    control(f"kill {h}","ok")
    exited(uart,h,3,0)
    control(f"reap {h}","code=0")
    spin = pid(uart,"run spin")
    control("mem","pending_io=0")
    control(f"kill {spin}","ok")
    exited(uart,spin,3,0)
    control(f"reap {spin}","code=0")
    uart.command("restart files","utility sessions revoked")
    output = control(f"revocation {c}","access=fenced")
    assert "discarded_staging=unknown effects=recovery-required" in output, output
    control(f"actor-status {c}","denied")
    uart.command("cat takeover-a","original")
    uart.command("cat takeover-b","untouched")
    assert counters(uart) == baseline
    # Fresh grants use a new service binding; old queued COMMIT cannot affect them.
    fresh = pid(uart,"session takeover-a takeover-b")
    actor(uart,fresh,"read"); actor(uart,fresh,"stage"); actor(uart,fresh,"commit")
    uart.command("cat takeover-a","session client edit")
    cleanup(uart,fresh)
    _, state = snapshot(data)
    assert state["files"][(4,"takeover-a")] == b"session client edit"
    assert state["files"][(4,"takeover-b")] == b"untouched"
    uart.command("rm takeover-a"); uart.command("rm takeover-b")
    assert counters(uart) == baseline
    return {"verified":True,"finite_stall_ticks":600,"unconfirmed_after_ticks":200,
            "late_ack":True,"indefinite_stall_restart":True,"queued_commit_revoked":True,
            "owner_commands":len(durations),"max_owner_command_seconds":round(max(durations),3),
            "tripwire_seconds":2,"reclaimed":True,"disk_sha256":state["selected_sha256"]}
