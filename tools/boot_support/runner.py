# SPDX-License-Identifier: Apache-2.0
"""Run only the R0 VM and distinguish guest results from harness failures."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import environment
from .image import build, OUTPUT
from .scenarios import EXPECTED, reached


def classify(returncode, timed_out, serial, build_id):
    if timed_out:
        return "timeout"
    if returncode == 33 and f"RUSTIC SUCCESS component=boot build={build_id}" in serial.splitlines() and "RUSTIC PANIC" not in serial and "RUSTIC FATAL" not in serial and "RUSTIC EXCEPTION" not in serial:
        return "success"
    if returncode == 35 and "RUSTIC PANIC" in serial:
        return "panic"
    if returncode == 37 and "RUSTIC FATAL" in serial:
        return "fatal"
    if returncode == 39 and f"RUSTIC EXCEPTION build={build_id} " in serial:
        return "exception"
    return "unexpected"


def run(image, timeout):
    from .block_evidence import MODES
    from . import block_runner
    metadata = json.loads((Path(image).resolve().parent / "image.json").read_text())
    if metadata["mode"] in MODES:
        return block_runner.run(image, timeout, run_once)
    return run_once(image, timeout)


def run_once(image, timeout, storage=(), output=None):
    image = Path(image).resolve()
    directory = output or image.parent
    metadata = json.loads((image.parent / "image.json").read_text())
    if environment.digest(image) != metadata["image_sha256"]:
        raise RuntimeError("image changed since construction")
    config = environment.CONFIG
    serial_path = directory / "serial.log"
    serial_path.write_text("")
    started = time.monotonic()
    timed_out = False
    with tempfile.TemporaryDirectory(prefix="rustic-ovmf-") as temporary:
        variables = Path(temporary) / "OVMF_VARS.fd"
        shutil.copyfile("/usr/share/OVMF/OVMF_VARS_4M.fd", variables)
        command = [
            "qemu-system-x86_64", "-machine", config["machine"],
            "-accel", config["accelerator"], "-cpu", config["cpu"], "-smp", "1", "-m", "256M",
            "-drive", "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd",
            "-drive", f"if=pflash,format=raw,file={variables}",
            "-drive", f"if=virtio,format=raw,readonly=on,file={image}",
            "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-display", "none", "-serial", f"file:{serial_path}", "-monitor", "none",
            "-nic", "none", "-no-reboot",
        ]
        command += list(storage)
        with (directory / "qemu.log").open("w") as log:
            process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
            try:
                returncode = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                process.kill()
                returncode = process.wait()
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
    serial = serial_path.read_text(errors="replace") if serial_path.exists() else ""
    outcome = classify(returncode, timed_out, serial, metadata["build_id"])
    result = {
        "outcome": outcome, "returncode": returncode, "timed_out": timed_out,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "timeout_seconds": timeout, "command": command,
        "build_id": metadata["build_id"], "image_sha256": metadata["image_sha256"],
    }
    (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"{metadata['mode']}: {outcome} ({result['elapsed_seconds']}s)", flush=True)
    return result


def suite(timeout):
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / "suite.json").unlink(missing_ok=True)
    results = []
    for mode, expected in EXPECTED.items():
        result = run(build(mode), timeout)
        serial = (OUTPUT / mode / "serial.log").read_text(errors="replace")
        # A loader failure that hangs cannot pass the deliberate-hang fixture.
        reached_fixture = reached(mode, serial)
        if result["outcome"] != expected or not reached_fixture:
            raise RuntimeError(f"{mode}: expected {expected}, got {result}; inspect {OUTPUT / mode}")
        results.append({"mode": mode, **result})
    (OUTPUT / "suite.json").write_text(json.dumps(results, indent=2) + "\n")
    print(f"All {len(EXPECTED)} boot scenarios verified.", flush=True)
