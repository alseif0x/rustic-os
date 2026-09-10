# SPDX-License-Identifier: Apache-2.0
"""Native owner job helpers and input/provisioning progress acceptance."""
import re
import time
from .cases import counters, pid
from .authority_cases import actor, cleanup

def start_restart(uart):
    return int(re.search(r"job=(\d+)",uart.command("restart files async","restart requested"))[1])

def wait_job(uart, job):
    deadline=time.monotonic()+15
    while True:
        value=uart.command(f"job-status {job}")
        if " complete " in value:
            assert "status=0" in value,value
            return value
        assert time.monotonic()<deadline,value
        time.sleep(.02)

def exercise(uart,data):
    baseline=counters(uart)
    uart.command("write progress-a original")
    uart.command("write progress-b untouched")
    uart.command("stall files 0","diagnostic armed")
    uart.send(b"cat progress-a\r")
    uart.until(b"cat progress-a\r\n")
    started=time.monotonic()
    uart.send(b"\x03echo typeahead-preserved\r")
    value=uart.until()
    assert "error: Interrupted" in value,value
    interrupted=time.monotonic()-started
    assert interrupted<2,interrupted
    value=uart.until()
    assert "typeahead-preserved" in value and "error:" not in value,value
    uart.commands+=2
    uart.command("cat progress-a","Protocol")
    uart.command("mem","pending_io=0")
    job=start_restart(uart)
    uart.command("services")
    wait_job(uart,job)
    uart.command("cat progress-a","original")
    uart.command("stall files 0","diagnostic armed")
    uart.send(b"cat progress-a\r");uart.until(b"cat progress-a\r\n")
    uart.send(b"x"*1100+b"\recho after-overflow\r")
    value=uart.until()
    assert "typeahead overflow" in value and "Interrupted" in value,value
    value=uart.until()
    assert "after-overflow" in value and "unknown command" not in value,value
    uart.commands+=2
    job=start_restart(uart);wait_job(uart,job)
    # Reading a retained result after a newer restart cannot rebind an old endpoint.
    newer=start_restart(uart)
    wait_job(uart,newer)
    uart.command(f"job-status {job}","complete")
    uart.command("cat progress-a","original")
    assert counters(uart)==baseline

    # C's actual COMMIT is admitted to the device; H shares the pending fence.
    c=pid(uart,"session progress-a progress-b")
    h=pid(uart,f"helper {c} progress-a progress-b")
    actor(uart,c,"read"); actor(uart,c,"stage")
    uart.command("hold-io 0 400","diagnostic armed")
    uart.command(f"act {c} commit","state=pending")
    deadline=time.monotonic()+3
    while "held=1" not in uart.command("io-status"):
        assert time.monotonic()<deadline,"device write never admitted"
        time.sleep(.01)
    uart.command(f"revoke {c}","access=requested")
    status=uart.command(f"revocation {h}","effects=unknown")
    assert "access=fenced" not in status,status
    job=start_restart(uart)
    pending=uart.command(f"job-status {job}","pending_io=1")
    assert "phase=2" in pending,pending
    uart.command("mem","pending_io=1")
    uart.command("echo owner-during-device-work","owner-during-device-work")
    wait_job(uart,job)
    uart.command(f"revocation {h}","access=fenced")
    uart.command(f"actor-status {c}","denied")
    uart.command("cat progress-a","original")
    uart.command("cat progress-b","untouched")
    assert counters(uart)==baseline
    fresh=pid(uart,"session progress-a progress-b")
    actor(uart,fresh,"read"); actor(uart,fresh,"stage"); actor(uart,fresh,"commit")
    cleanup(uart,fresh)
    from .oracle import snapshot
    _,state=snapshot(data)
    assert state["files"][(4,"progress-a")]==b"session client edit"
    assert state["files"][(4,"progress-b")]==b"untouched"
    uart.command("rm progress-a");uart.command("rm progress-b")
    assert counters(uart)==baseline
    return {"verified":True,"interrupt_seconds":round(interrupted,3),"typeahead":True,
            "stale_job_binding_rejected":True,"admitted_client_commit":True,
            "restart_pending_io":True,"reclaimed":True,"disk_sha256":state["selected_sha256"]}
