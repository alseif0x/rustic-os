# SPDX-License-Identifier: Apache-2.0
"""Own two disposable native VMs, their disk and evidence for one repetition."""
import json
from pathlib import Path
import subprocess
import tempfile
import time
import environment
from boot_support.runner import run_once
from boot_support.scenarios import reached, records
from terminal_support.machine import disk, machine
from terminal_support.connection import Connection
from terminal_support.oracle import snapshot
from . import provenance, workload


def collect(images, output, warmup, injected_ticks):
    output.mkdir()
    metrics = {}
    result = {"warmup": warmup, "status": "invalid", "metrics": metrics,
              "host_observation_before": provenance.observation(), "vm_boots_started": 0}
    try:
        probe = output / "probe"
        probe.mkdir()
        def started_probe():
            result["vm_boots_started"] += 1

        diagnostic = run_once(images["ok"], 60, output=probe, on_start=started_probe)
        serial = (probe / "serial.log").read_text()
        if diagnostic["outcome"] == "timeout":
            raise TimeoutError("kernel fixture timed out")
        assert diagnostic["outcome"] == "success" and reached("ok", serial), "invalid native kernel fixture"
        memory, = records(serial, "RUSTIC MEMORY ")
        processes, = records(serial, "RUSTIC PROCESS_MEMORY ")
        metrics.update(kernel_page_table_frames=int(memory["table_frames"]),
                       allocator_metadata_bytes=int(memory["metadata_bytes"]),
                       manager_metadata_bytes=int(processes["metadata_bytes"]),
                       process_fixture_peak_frames=int(processes["peak_frames"]),
                       kernel_load_bytes=provenance.load_bytes(images["terminal"].parent / "kernel.elf"))
        with tempfile.TemporaryDirectory(prefix="rustic-measure-") as temporary:
            temporary = Path(temporary)
            with disk(temporary / "data.raw", True) as data:
                sock = temporary / "uart.sock"
                started = time.perf_counter()
                with machine(images["terminal"], data, f"unix:{sock},server=on,wait=off", output / "qemu.log") as vm:
                    result["vm_boots_started"] += 1
                    uart = Connection(sock, vm, output / "serial.log", 60)
                    try:
                        uart.until()
                        metrics["boot_ready_seconds"] = time.perf_counter() - started
                        resident = workload.exercise(uart, metrics, injected_ticks)
                        workload.shutdown(uart, vm, resident, metrics)
                    finally:
                        uart.close()
                selected, state = snapshot(data)
                (output / "files.bin").write_bytes(selected)
                result["disk_sha256"] = state["selected_sha256"]
                assert state["files"] == {(3, "owner-policy"): b"rustic-owner-v1\nhelpers=explicit\n",
                                          (4, "a"): b"owner", (4, "b"): b"untouched"}, "unexpected persisted file set/content"
        result["status"] = "success"
    except (TimeoutError, subprocess.TimeoutExpired) as error:
        result.update(status="timeout", error=str(error)[:2000])
    except (AssertionError, RuntimeError, OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        result.update(status="timeout" if "UART timeout" in str(error) else "invalid",
                      error=f"{type(error).__name__}: {error}"[:2000])
    result["host_observation_after"] = provenance.observation()
    result["artifacts"] = {str(p.relative_to(output)): environment.digest(p)
                           for p in sorted(output.rglob("*")) if p.is_file()}
    (output / "sample.json").write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
    return result
