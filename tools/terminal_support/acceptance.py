# SPDX-License-Identifier: Apache-2.0
"""Trusted UART driver, two VM boots, independent disk oracle, bounded output."""
import json
from pathlib import Path
import tempfile
import time
from boot_support.image import package
from .machine import machine, disk
from .connection import Connection
from .cases import exercise
from .authority_cases import exercise as authority_exercise
from .oracle import inspect

def verify(image, timeout=60, output=None):
    image = Path(image).resolve()
    output = Path(output or image.parent)
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    # This second boot reuses the exact ELF; it never invokes candidate host code.
    mount = package(image.parent / "kernel.elf", "terminal", metadata["build_id"], {})
    cases, serials, logs = 0, [], []
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-test-") as temporary:
            temporary = Path(temporary)
            with disk(temporary / "data.raw", True) as data:
                for phase, boot in enumerate((image, mount), 1):
                    sock = temporary / f"uart-{phase}.sock"
                    transcript, log = output / f"serial-{phase}.log", output / f"qemu-{phase}.log"
                    serials.append(transcript)
                    logs.append(log)
                    with machine(boot, data, f"unix:{sock},server=on,wait=off", log) as vm:
                        uart = Connection(sock, vm, transcript, timeout)
                        try:
                            uart.until()
                            if phase == 1:
                                cases = exercise(uart)
                                authority = authority_exercise(uart, data)
                                cases = uart.commands
                            else:
                                uart.command("cat hello", "Hello from native Rust")
                                uart.command("cat /config/owner-policy", "helpers=explicit")
                                uart.command("mem", "processes=3 channels=4 pending_io=0")
                            uart.send(b"exit\r")
                            uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
                            if vm.wait(timeout=10) != 33:
                                raise RuntimeError("unclean terminal exit")
                        finally:
                            uart.close()
                    selected, oracle = inspect(data)
                allocation = data.stat().st_blocks * 512
        (output / "files.bin").write_bytes(selected)
        evidence = {"verified":True,"boots":2,"commands_phase_one":cases,"oracle":oracle,"allocated_bytes":allocation,
                    "kernel_sha256":metadata["kernel_sha256"],"build_id":metadata["build_id"],"authority":authority}
        (output / "terminal.json").write_text(json.dumps(evidence,indent=2)+"\n")
        result = {"outcome":"success","returncode":33,"timed_out":False,"elapsed_seconds":round(time.monotonic()-started,3),
                  "build_id":metadata["build_id"],"image_sha256":metadata["image_sha256"],"terminal":evidence}
        (output / "result.json").write_text(json.dumps(result,indent=2)+"\n")
        print(f"Native terminal: {cases} completed shell commands, two boots, real UART and independent persistent-file oracle.",flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(b"\n".join(p.read_bytes() for p in serials if p.exists()))
        (output / "qemu.log").write_bytes(b"\n".join(p.read_bytes() for p in logs if p.exists()))
