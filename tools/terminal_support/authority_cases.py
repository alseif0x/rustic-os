# SPDX-License-Identifier: Apache-2.0
"""Native C/H authority mission through owner UART; independent persistent bytes."""
import re
import time
from .cases import pid, counters, exited
from .oracle import snapshot

def actor(uart, child, action, status=0):
    output = uart.command(f"act {child} {action}", f"actor status={status} ")
    return {k:int(v) for k,v in re.findall(r"(status|value|other|control_denied|version)=(\d+)", output)}

def cleanup(uart, *children):
    for child in children:
        uart.command(f"kill {child}", "ok")
        exited(uart, child, 3, 0)
        uart.command(f"reap {child}", "code=0")

def exercise(uart, data):
    baseline = counters(uart)
    uart.command("write authority-a before")
    uart.command("write authority-b untouched")
    c = pid(uart, "session authority-a authority-b")
    # Failed helper provisioning must release its dormant process and both channels.
    before = counters(uart)
    uart.command(f"helper {c} authority-b authority-a", "denied")
    assert counters(uart) == before
    h = pid(uart, f"helper {c} authority-a authority-b")
    uart.command(f"helper {h} authority-a authority-b", "denied")
    for child in (c, h):
        read = actor(uart, child, "read")
        assert read["value"] == 6 and read["other"] == 17 and read["control_denied"] == 1, read
    actor(uart, h, "stage", 17)
    actor(uart, c, "stage")
    actor(uart, c, "commit")
    uart.command("cat authority-a", "session client edit")
    actor(uart, c, "read")
    actor(uart, c, "stage")
    uart.command("write authority-a human-edit")
    actor(uart, c, "commit", 13)
    uart.command("cat authority-a", "human-edit")
    actor(uart, c, "read")
    actor(uart, c, "stage")
    fills = [actor(uart, child, "flood") for child in (c,h)]
    assert all(r["value"] >= 4 and r["other"] > 0 for r in fills), fills
    active = counters(uart)
    assert active["processes"] == 5 and active["channels"] == 8 and active["pending_io"] == 0, active
    start = time.monotonic()
    uart.command("write authority-a owner-under-pressure")
    write_seconds = time.monotonic() - start
    start = time.monotonic()
    uart.command(f"revoke {h}", "access=fenced members=2 discarded_staging=1 effects=settled")
    revoke_seconds = time.monotonic() - start
    for child in (c,h):
        drained = actor(uart, child, "drain")
        assert drained["other"] >= 1, drained # Queued requests rechecked after the fence.
        actor(uart, child, "read", 18)
        uart.command(f"permissions {child}", "rights=0")
    actor(uart, c, "commit", 18)
    uart.command("cat authority-a", "owner-under-pressure")
    cleanup(uart,c,h)
    assert counters(uart) == baseline

    # Fresh issuance does not restore old PIDs or root references. A moved handle is
    # still bound to C's authenticated peer; it cannot lend C's write grant to H.
    c = pid(uart, "session authority-a authority-b")
    h = pid(uart, f"helper {c} authority-a authority-b")
    uart.command(f"move-check {c} {h}", "actor status=17 ")
    assert actor(uart,c,"stale")["value"] == 1
    actor(uart,h,"read")
    uart.command(f"revoke {c}", "access=fenced members=2")
    actor(uart,h,"read",18)
    cleanup(uart,c,h)
    # Root death must also fence H when H still holds the moved endpoint.
    c = pid(uart, "session authority-a authority-b")
    h = pid(uart, f"helper {c} authority-a authority-b")
    uart.command(f"move-check {c} {h}", "actor status=17 ")
    uart.command(f"kill {c}", "ok")
    exited(uart,c,3,0)
    actor(uart,h,"read",18)
    uart.command(f"reap {c}", "code=0")
    cleanup(uart,h)
    assert counters(uart) == baseline

    # One inherited absolute deadline, then service restart removes all actors.
    c = pid(uart, "session authority-a authority-b 300")
    h = pid(uart, f"helper {c} authority-a authority-b")
    def expiry(child):
        return int(re.search(r"expires=(\d+)",uart.command(f"permissions {child}"))[1])
    assert expiry(c) == expiry(h) != 0
    time.sleep(3.1)
    actor(uart,c,"read",19)
    actor(uart,h,"read",19)
    uart.command("restart files", "utility sessions revoked")
    uart.command(f"act {c} read", "denied")
    uart.command(f"helper {c} authority-a authority-b", "denied")
    assert counters(uart) == baseline
    uart.command("cat authority-a", "owner-under-pressure")
    uart.command("cat authority-b", "untouched")
    _, state = snapshot(data)
    assert state["files"][(4,"authority-a")] == b"owner-under-pressure"
    assert state["files"][(4,"authority-b")] == b"untouched"
    uart.command("rm authority-a")
    uart.command("rm authority-b")
    return {"verified":True,"client_helper":True,"queued_revocation":True,"moved_handle":True,
            "root_death_after_move":True,"inherited_expiry":True,"restart":True,"owner_pressure_write_seconds":round(write_seconds,3),
            "revoke_seconds":round(revoke_seconds,3),"peak_processes":5,"peak_channels":8,
            "independent_disk_sha256":state["selected_sha256"],"resources_reclaimed":True}
