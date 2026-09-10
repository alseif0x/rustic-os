# SPDX-License-Identifier: Apache-2.0
"""Run fixed native latency cases; preserve evidence before disposable disks disappear."""
import json
from pathlib import Path
import re
import select
import shutil
import sys
import tempfile
import time

import environment
from ..connection import Connection
from ..machine import disk
from ..oracle import snapshot
from . import lab
from .evidence import CASES, SUSPENDED, require, stat, validate


def _text(path):
    with path.open("rb") as stream:
        value = stream.read(1024 * 1024 + 1)
    require(len(value) <= 1024 * 1024, "latency log exceeds evidence budget")
    return value.decode("ascii", "backslashreplace")


def _drain(uart):
    # Bounded nonblocking collection keeps diagnostics emitted during reset in
    # the evidence even when the guest cannot yet send its final UART prompt.
    while select.select([uart.socket], [], [], 0)[0]:
        chunk = uart.socket.recv(4096)
        if not chunk:
            raise RuntimeError("UART closed during delayed completion")
        uart.data.extend(chunk)
        uart.pending.extend(chunk)
        require(len(uart.data) <= 1024 * 1024, "UART evidence exceeds budget")
    uart.save()


def _exercise(uart, vm, monitor, output, case, result):
    hold, expired = CASES[case]
    uart.until()
    uart.command("write hello before", "written 6 bytes")
    uart.command("write other untouched", "written 9 bytes")
    result["before"] = uart.command("stat hello")
    before = stat(result["before"])
    token_reply = uart.command("retry-key hello 991")
    tokens = re.findall(r"^retry-key=([0-9a-f]{64})$", "\n".join(token_reply.splitlines()), re.MULTILINE)
    require(len(tokens) == 1, "missing or ambiguous retry key")
    token = tokens[0]
    result["resources_before"] = uart.command("mem")
    monitor.breakpoint()
    started = time.monotonic()
    uart.send(f'replace hello {before["version"]} {token} "after"\r'.encode("ascii"))
    deadline = started + 5
    while SUSPENDED not in _text(output / "backend.log").splitlines():
        if vm.poll() is not None or time.monotonic() >= deadline:
            raise RuntimeError("real FLUSH breakpoint was not observed")
        _drain(uart)
        time.sleep(.01)
    suspended = time.monotonic()
    result["suspension_observed_seconds"] = suspended - started
    deadline = suspended + hold
    while time.monotonic() < deadline:
        _drain(uart)
        time.sleep(min(.02, max(0, deadline - time.monotonic())))
    _drain(uart)
    result["uart_before_resume"] = uart.pending.decode("ascii", "backslashreplace")
    monitor.breakpoint(resume=True)
    result["suspended_seconds"] = time.monotonic() - suspended
    result["reply"] = uart.until()
    result["reply_seconds"] = time.monotonic() - started
    result["resources_after_completion"] = uart.command("mem")
    if expired:
        require("error: Uncertain" in result["reply"].splitlines(), "expired mutation did not report Uncertain")
        result["restart"] = uart.command("restart files", "utility sessions revoked")
    result["content"] = uart.command("cat hello")
    result["receipt"] = uart.command(f'receipt {before["id"]} {token}',
                                     "OutcomeUnknown" if expired else "committed id=")
    result["other"] = uart.command("cat other")
    result["after"] = uart.command("stat hello")
    result["resources_final"] = uart.command("mem")
    uart.send(b"exit\r")
    uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
    require(vm.wait(timeout=10) == 33, "unclean latency guest exit")
    result["clean_exit"] = True


def _capture(data, output, result):
    with data.open("rb") as stream:
        prefix = stream.read(174 * 512)
        stream.seek(-512, 2)
        tail = stream.read(512)
    for name, payload in (("files.bin", prefix), ("last-sector.bin", tail)):
        path = output / name
        path.write_bytes(payload)
        result[name] = {"bytes": len(payload), "sha256": environment.digest(path)}


def _case(image, output, case, timeout, metadata):
    output.mkdir()
    result = {"verified": False, "case": case, "requested_hold_seconds": CASES[case][0],
              "expected_budget_ticks": 500, "image_sha256": metadata["image_sha256"],
              "kernel_sha256": metadata["kernel_sha256"], "build_id": metadata["build_id"],
              "cleanup_confirmed": False}
    temporary = None
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-latency-") as temporary:
            temporary = Path(temporary)
            with disk(temporary / "data.raw", True) as data:
                try:
                    with lab.running(image, data, temporary, output, result) as (vm, monitor, uart_path):
                        uart = Connection(uart_path, vm, output / "serial.log", timeout,
                                          output / "commands.jsonl")
                        try:
                            _exercise(uart, vm, monitor, output, case, result)
                        finally:
                            uart.close()
                    _, state = snapshot(data)
                    validate(case, result, state, _text(output / "backend.log"), _text(output / "serial.log"))
                    result["disk_sha256"] = state["selected_sha256"]
                finally:
                    # running() has stopped both processes, including on failure.
                    # Retain fixed raw sectors even when the oracle rejects them.
                    original_error = sys.exception()
                    try:
                        require(result.get("process_cleanup_confirmed") is True,
                                "owned process cleanup failed; cannot capture a stopped disk")
                        _capture(data, output, result)
                    except Exception as capture:
                        result["capture_error"] = f"{type(capture).__name__}: {capture}"[:2000]
                        if original_error is None:
                            raise
                        original_error.add_note("Disk evidence capture failed: " + result["capture_error"])
        result["cleanup_confirmed"] = True
        result["verified"] = True
    except Exception as error:
        result["cleanup_confirmed"] = (result.get("process_cleanup_confirmed") is True
                                       and temporary is not None and not temporary.exists())
        result["error"] = f"{type(error).__name__}: {error}"[:2000]
        raise
    finally:
        (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def _preserve_image(image, output):
    original = Path(image).resolve()
    metadata = json.loads(_text(original.parent / "image.json"))
    require(metadata.get("mode") == "recovery-test" and metadata.get("environment") == environment.CONFIG,
            "latency requires a reference recovery-test image")
    require(re.fullmatch("[0-9a-f]{16}", metadata.get("build_id", "")) is not None, "invalid build identity")
    for name, field in ((original.name, "image_sha256"), ("kernel.elf", "kernel_sha256")):
        expected = metadata.get(field, "")
        require(re.fullmatch("[0-9a-f]{64}", expected) is not None, "invalid image provenance digest")
        source, target = original.parent / name, output / ("rustic-os.img" if field == "image_sha256" else name)
        require(environment.digest(source) == expected, "source image/kernel hash mismatch")
        shutil.copyfile(source, target)
        require(environment.digest(target) == expected, "input changed while preserving image")
    (output / "image.json").write_text(json.dumps(metadata, indent=2) + "\n")
    return output / "rustic-os.img", metadata


def verify(image, output, timeout=60):
    require(1 <= timeout <= 120, "UART timeout is outside supported bounds")
    output = Path(output).resolve()
    output.mkdir(parents=True)  # An existing evidence directory is never reused.
    summary = {"verified": False, "guest_boots": 0, "storage_processes": 0,
               "mechanism": "qmp_qemu_io_blkdebug_flush_to_disk",
               "host_storage": "separate_qemu_unix_nbd", "cases": [], "cleanup_confirmed": False}
    try:
        environment.verify()
        image, metadata = _preserve_image(image, output)
        summary.update({key: metadata[key] for key in ("image_sha256", "kernel_sha256", "build_id")})
        for case in CASES:
            result = _case(image, output / case, case, timeout, metadata)
            summary["cases"].append(result)
            summary["guest_boots"] += 1
            summary["storage_processes"] += 1
        require(environment.digest(image) == metadata["image_sha256"], "preserved boot image changed during run")
        summary["cleanup_confirmed"] = all(item["cleanup_confirmed"] for item in summary["cases"])
        summary["verified"] = summary["cleanup_confirmed"]
    except Exception as error:
        summary["error"] = f"{type(error).__name__}: {error}"[:2000]
        raise
    finally:
        (output / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"Native FLUSH latency: 2 cases, 2 guest boots, 2 separate storage processes; evidence {output}", flush=True)
    return summary
