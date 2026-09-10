# SPDX-License-Identifier: Apache-2.0
"""Integration acceptance: real compilation, guest execution, cancellation and quotas."""
import json
import subprocess
import sys
import time

from .fixtures import candidate
from .jobs import JOBS, execute, cancel
from .prepare import ROOT
from .probes import probe
from boot_support.scenarios import reached, MEMORY_FAULTS, BLOCK_MODES


def suite(revision):
    evidence = {"revision": revision, "isolation": probe(), "cases": []}
    cases = [("ok", revision, "ok", 120, 30, "success"),
             ("panic", revision, "panic", 120, 30, "boot_failed"),
             ("hang", revision, "hang", 120, 15, "boot_timeout"),
             ("invalid", revision, "invalid", 120, 30, "boot_failed"),
             ("exception", revision, "exception", 120, 30, "boot_failed"),
             ("gp", revision, "gp", 120, 30, "boot_failed"),
             ("doublefault", revision, "doublefault", 120, 30, "boot_failed"),
             ("timer-stall", revision, "timer-stall", 120, 15, "boot_timeout"),
             ("compile_error", candidate(revision, 'compile_error!("RUSTIC_COMPILE_FIXTURE");\nfn main() {}\n'),
              "ok", 120, 30, "build_failed"),
             ("build_hang", candidate(revision, 'fn main() { eprintln!("RUSTIC_BUILD_HANG"); loop { std::thread::sleep(std::time::Duration::from_secs(1)); } }\n'),
              "ok", 15, 30, "build_timeout")]
    cases[8:8] = [(mode, revision, mode, 120, 30, "boot_failed") for mode in MEMORY_FAULTS]
    cases.insert(0, ("terminal-test", revision, "terminal-test", 120, 60, "success"))
    cases.insert(0, ("recovery-test", revision, "recovery-test", 120, 60, "success"))
    cases[13:13] = [(mode, revision, mode, 120, 30, "success") for mode in BLOCK_MODES]
    for name, commit, mode, build_seconds, boot_seconds, expected in cases:
        result = execute(commit, mode, build_seconds, boot_seconds)
        if result["status"] != expected:
            raise RuntimeError(f"{name}: expected {expected}, got {result['status']}; job {result['job_id']}")
        directory = JOBS / result["job_id"]
        if (name in ("ok", "exception", "gp", "doublefault", "timer-stall", "terminal-test", "recovery-test") or name in MEMORY_FAULTS or name in BLOCK_MODES) and not reached(mode, (directory / "serial.log").read_text()):
            raise RuntimeError(name + ": guest did not verify the expected interrupt fixture")
        marker = {"compile_error": ("build.log", "RUSTIC_COMPILE_FIXTURE"),
                  "build_hang": ("build.log", "RUSTIC_BUILD_HANG"),
                  "hang": ("serial.log", "RUSTIC HANG deliberate=1")}.get(name)
        if marker and marker[1] not in (directory / marker[0]).read_text():
            raise RuntimeError(name + ": failed before reaching fixture")
        evidence["cases"].append({"case": name, "job_id": result["job_id"], "status": result["status"]})
        print(json.dumps(evidence["cases"][-1]), file=sys.stderr, flush=True)
    evidence["cases"].append(cancellation(cases[-1][1]))
    # A fresh workspace must work again after the deliberately killed build.
    repeat = execute(revision, "ok", 120, 30)
    if repeat["status"] != "success" or not reached("ok", (JOBS / repeat["job_id"] / "serial.log").read_text()):
        raise RuntimeError("clean repetition failed")
    evidence["cases"].append({"case": "clean_repeat", "job_id": repeat["job_id"], "status": repeat["status"]})
    (ROOT / "artifacts/sandbox-suite.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return {"status": "success", **evidence}


def cancellation(revision):
    output = ROOT / "artifacts/cancel-output.json"
    events = ROOT / "artifacts/cancel-events.log"
    with output.open("w") as stdout, events.open("w") as stderr:
        process = subprocess.Popen([sys.executable, str(ROOT / "tools/sandbox.py"), "run",
                                    "--revision", revision, "--build-timeout", "120"],
                                   stdout=stdout, stderr=stderr)
        job_id = None
        try:
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline and process.poll() is None:
                lines = events.read_text().splitlines()
                if lines:
                    job_id = json.loads(lines[0])["job_id"]
                    log = JOBS / job_id / "build.log"
                    if log.exists() and "RUSTIC_BUILD_HANG" in log.read_text():
                        break
                time.sleep(0.2)
            else:
                raise RuntimeError("cancellation fixture never reached running build")
            cancel(job_id)
            process.wait(timeout=30)
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                if job_id:
                    cancel(job_id)
    result = json.loads(output.read_text())
    if result["status"] != "cancelled" or result["cleanup_errors"]:
        raise RuntimeError("cancellation or cleanup failed")
    return {"case": "cancellation", "job_id": job_id, "status": result["status"]}
