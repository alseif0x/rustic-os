# SPDX-License-Identifier: Apache-2.0
"""Exercise the declared V5 process, IPC, client and child-slot topology."""
import json
from pathlib import Path
import re
import tempfile
import time

from .cases import pid
from .connection import Connection
from .failure import preserve_failure
from .machine import disk, machine


def _mem(uart):
    text = uart.command("mem")
    values = {key: int(value) for key, value in re.findall(r"\b(processes|channels)=(\d+)", text)}
    if set(values) != {"processes", "channels"}:
        raise ValueError(f"unrecognized mem output: {text!r}")
    return values


def _limits(uart):
    text = uart.command("limits")

    def pair(label, usage="used"):
        match = re.search(rf"(?m)^{label} {usage}=(\d+)/(\d+)", text)
        if not match:
            raise ValueError(f"missing {label} in limits output: {text!r}")
        return tuple(map(int, match.groups()))

    return {
        "processes": pair("processes"),
        "channels": pair("channels"),
        "endpoint_handles": pair("endpoint_handles"),
        "owner_handles_max": pair("owner_handles", "max"),
        "file_clients": pair("file_clients"),
        "child_slots": pair("child_slots"),
    }


def _snapshot(uart):
    memory = _mem(uart)
    limits = _limits(uart)
    if any(memory[key] != limits[key][0] for key in memory):
        raise AssertionError(f"mem/limits process or channel counts diverged: {memory}, {limits}")
    return {
        "processes": memory["processes"],
        "channels": memory["channels"],
        "endpoint_handles": limits["endpoint_handles"][0],
        "owner_handles_max": limits["owner_handles_max"][0],
        "file_clients": limits["file_clients"][0],
        "child_slots": limits["child_slots"][0],
    }


def _reap_all(uart, children):
    for child in children:
        uart.command(f"kill {child}", "ok")
    deadline = time.monotonic() + 10
    while True:
        text = uart.command("ps")
        states = {
            int(pid): state
            for pid, state in re.findall(
                r"(?m)^(\d+) (dormant|ready|running|blocked|exited) ", text
            )
        }
        if all(states.get(child) == "exited" for child in children):
            break
        if time.monotonic() >= deadline:
            raise AssertionError(f"children did not exit after kill: {states}")
        time.sleep(0.05)
    for child in children:
        uart.command(f"reap {child}", "ok exit_kind=")


def _fill(uart):
    children = [pid(uart, "run watch hello") for _ in range(2)]
    children.extend(pid(uart, "run spin") for _ in range(4))
    return children


def _assert_full(before, full):
    expected = {
        "processes": before["processes"] + 6,
        "channels": before["channels"] + 8,
        "endpoint_handles": before["endpoint_handles"] + 16,
        "file_clients": 4,
        "child_slots": 6,
    }
    for key, value in expected.items():
        if full[key] != value:
            raise AssertionError(f"full topology {key}={full[key]}, expected {value}; {full}")
    if full["processes"] <= 8 and full["channels"] <= 8:
        raise AssertionError(f"topology did not exceed the former bootstrap capacity: {full}")


def verify(image, initialize_image=None, output=None, timeout=60):
    image = Path(image).resolve()
    initialize_image = Path(initialize_image).resolve() if initialize_image else None
    output = Path(output or image.parent / "capacity-topology").resolve()
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    serials, logs = [], []
    evidence = {"verified": False, "build_id": metadata["build_id"], "kernel_sha256": metadata["kernel_sha256"]}
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-capacity-topology-") as temporary:
            temporary = Path(temporary)
            with disk(temporary / "capacity-volume.raw", True) as data, preserve_failure(data, output, "capacity-topology", metadata):
                boots = []
                if initialize_image is not None:
                    boots.append((initialize_image, "initialize"))
                boots.append((image, "topology"))
                for boot_image, phase in boots:
                    serial = output / f"capacity-topology-{phase}.serial.log"
                    log = output / f"capacity-topology-{phase}.qemu.log"
                    serials.append(serial)
                    logs.append(log)
                    sock = temporary / f"{phase}.uart.sock"
                    with machine(boot_image, data, f"unix:{sock},server=on,wait=off", log) as vm:
                        uart = Connection(sock, vm, serial, timeout, output / f"capacity-topology-{phase}.commands.jsonl")
                        try:
                            uart.until()
                            uart.command("services", "mounted")
                            if phase == "initialize":
                                uart.send(b"exit\r")
                                uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
                                if vm.wait(timeout=10) != 33:
                                    raise RuntimeError("unclean V5 volume initialization exit")
                                continue

                            uart.command('write hello "capacity topology"', "written 17 bytes")
                            uart.command("cat hello", "capacity topology")
                            initial = _snapshot(uart)
                            printed = uart.command("limits")
                            declared = {
                                "processes": (16, "processes"),
                                "channels": (24, "channels"),
                                "endpoint_handles": (64, "endpoint_handles"),
                                "owner_handles_max": (24, "owner_handles"),
                                "file_clients": (4, "file_clients"),
                                "child_slots": (6, "child_slots"),
                            }
                            for key, (limit, label) in declared.items():
                                found = re.search(rf"(?m)^{label} (?:used|max)=\d+/(\d+)", printed)
                                if not found or int(found[1]) != limit:
                                    raise AssertionError(f"{label} limit was not declared as {limit}: {printed!r}")

                            cycles = []
                            for cycle in range(2):
                                children = _fill(uart)
                                full = _snapshot(uart)
                                _assert_full(initial, full)
                                uart.command("run spin", "child slot capacity exhausted")
                                after_refusal = _snapshot(uart)
                                if after_refusal != full:
                                    raise AssertionError(f"refused launch changed live counts: {full} -> {after_refusal}")
                                uart.command("echo shell-usable", "shell-usable")
                                uart.command("cat hello", "capacity topology")
                                _reap_all(uart, children)
                                after_reap = _snapshot(uart)
                                if after_reap != initial:
                                    raise AssertionError(f"kill/reap changed baseline counts: {initial} -> {after_reap}")
                                uart.command("cat hello", "capacity topology")
                                cycles.append({
                                    "cycle": cycle + 1,
                                    "full": full,
                                    "refusal": "child slot capacity exhausted",
                                    "refusal_counts_unchanged": True,
                                    "after_reap": after_reap,
                                })

                            children = _fill(uart)
                            full = _snapshot(uart)
                            _assert_full(initial, full)
                            uart.command('write capacity-note "shell alive"', "written 11 bytes")
                            uart.command("cat capacity-note", "shell alive")
                            process_table = uart.command("ps", "PROGRAM")
                            if not all(re.search(rf"(?m)^{child} ", process_table) for child in children):
                                raise AssertionError("ps did not retain every admitted child while full")
                            uart.command("restart files", "files restarted; utility sessions revoked")
                            uart.command("cat hello", "capacity topology")
                            after_restart = _snapshot(uart)
                            if after_restart != initial:
                                raise AssertionError(f"restart files failed to restore baseline: {initial} -> {after_restart}")
                            uart.command("rm capacity-note")
                            uart.send(b"exit\r")
                            uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
                            if vm.wait(timeout=10) != 33:
                                raise RuntimeError("unclean capacity topology terminal exit")
                            evidence.update({
                                "verified": True,
                                "declared_limits": {key: value[0] for key, value in declared.items()},
                                "baseline": initial,
                                "manual_fill_reap_cycles": cycles,
                                "recovery": {
                                    "full": full,
                                    "restart_files_succeeded": True,
                                    "after_restart": after_restart,
                                },
                                "commands": uart.commands,
                                "boots": len(boots),
                            })
                        finally:
                            uart.close()
        (output / "capacity-topology.json").write_text(json.dumps(evidence, separators=(",", ":")) + "\n")
        print("Capacity topology: six mixed-role children, no-leak refusal, repeated cleanup and full-pool recovery verified.", flush=True)
        return evidence
    finally:
        (output / "serial.log").write_bytes(b"\n".join(path.read_bytes() for path in serials if path.exists()))
        (output / "qemu.log").write_bytes(b"\n".join(path.read_bytes() for path in logs if path.exists()))
