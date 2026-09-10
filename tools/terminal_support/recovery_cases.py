# SPDX-License-Identifier: Apache-2.0
"""Native receipt, replay, conflict, quota, identity and lost-response assertions."""
import re
from .cases import pid, exited, counters

def stat(uart, name="hello"):
    return {k:int(v) for k,v in re.findall(r"(id|version|bytes)=(\d+)",uart.command(f"stat {name}"))}

def key(uart, number, name="hello"):
    return re.search(r"retry-key=([0-9a-f]{64})", uart.command(f"retry-key {name} {number}"))[1]

def exercise(uart, capture):
    uart.command("write hello before")
    uart.command("write other untouched")
    old = stat(uart)
    baseline = counters(uart)
    capture(old, key(uart, 42))
    lost = key(uart, 77)
    child = pid(uart, "run lost-reply hello other")
    exited(uart, child, 1, 0)
    uart.command(f"permissions {child}", "other=17")
    uart.command(f"reap {child}", "code=0")
    uart.command(f"receipt {old['id']} {lost}", "committed id=")
    uart.command("cat hello", "reply deliberately unobserved")
    uart.command("write hello human-edit")
    human = stat(uart)
    uart.command(f'replace hello {old["version"]} {lost} "reply deliberately unobserved"', "committed id=")
    assert stat(uart) == human
    uart.command("cat hello", "human-edit")
    uart.command(f'replace hello {old["version"]} {lost} "different"', "IdempotencyConflict")
    uart.command(f'replace other {old["version"]} {lost} "reply deliberately unobserved"', "IdempotencyConflict")
    wrong = ("01" if lost[:2] != "01" else "02") + lost[2:]
    uart.command(f"receipt {old['id']} {wrong}", "Lineage")
    missing = key(uart, 88)
    uart.command(f"receipt {old['id']} {missing}", "OutcomeUnknown")
    second = key(uart, 78)
    uart.command(f'replace hello {old["version"]} {second} "stale"', "Version")
    uart.command(f'replace hello {human["version"]} {second} ""', "bytes=0")
    empty = stat(uart)
    uart.command(f'replace hello {empty["version"]} {missing} "full"', "Full")
    assert stat(uart) == empty
    uart.command("rotate-receipts", "epoch=2")
    uart.command(f"receipt {old['id']} {lost}", "ExpiredEpoch")
    uart.command(f'replace hello {empty["version"]} {lost} "stale-key"', "ExpiredEpoch")
    final = key(uart, 42)
    uart.command(f'replace hello {empty["version"]} {final} "after"', "committed id=")
    uart.command("restart files", "utility sessions revoked")
    uart.command(f"receipt {old['id']} {final}", "committed id=")
    assert counters(uart) == baseline
    return {"id": old["id"], "version": empty["version"], "key": final}

def verify_final(uart, final):
    uart.command(f'receipt {final["id"]} {final["key"]}', "committed id=")
    before = stat(uart)
    uart.command(f'replace hello {final["version"]} {final["key"]} "after"', "committed id=")
    assert stat(uart) == before
    uart.command("cat hello", "after")
    uart.command("cat other", "untouched")
