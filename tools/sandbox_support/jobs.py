# SPDX-License-Identifier: Apache-2.0
"""Resolve immutable revisions; coordinate two sandboxes and owner-side evidence."""
import fcntl
import hashlib
import json
from pathlib import Path
import re
import signal
import subprocess
import time
import uuid

from . import runtime
from .artifacts import collect, sha256
from .prepare import ROOT, STATE
from .process import command
from boot_support.scenarios import MODES

JOBS = ROOT / "artifacts/jobs"


def write_state(directory, state):
    temporary = directory / "job.json.tmp"
    temporary.write_text(json.dumps(state, indent=2) + "\n")
    temporary.replace(directory / "job.json")


def cancel(job_id):
    if not re.fullmatch(r"[0-9a-f]{32}", job_id):
        raise ValueError("invalid job id")
    directory = JOBS / job_id
    if not (directory / "job.json").is_file():
        raise ValueError("unknown job")
    (directory / "cancel.request").touch()
    for phase in ("build", "boot"):
        runtime.remove_owned(job_id, phase, directory)
    with (ROOT / ".cache/sandbox.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return {"status": "cancellation_requested", "job_id": job_id}
        state = json.loads((directory / "job.json").read_text())
        if state["status"] in ("preparing", "build_running", "boot_running", "cleanup_failed"):
            state.update(status="cancelled", cleanup_errors=[], finished_at=time.time())
            write_state(directory, state)
    return {"status": "cancellation_requested", "job_id": job_id}


def execute(revision, mode, build_timeout, boot_timeout):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be a full local commit SHA")
    if mode not in MODES:
        raise ValueError("unsupported mode")
    if not 1 <= build_timeout <= 300 or not 1 <= boot_timeout <= 120:
        raise ValueError("timeouts outside supported limits")
    actual = subprocess.check_output(["git", "rev-parse", "--verify", revision + "^{commit}"],
                                     cwd=ROOT, text=True).strip()
    if actual != revision:
        raise ValueError("revision must identify a commit directly")
    config = json.loads(STATE.read_text())
    image = config["image"]
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", image):
        raise ValueError("prepare a trusted immutable toolchain image first")
    lock_path = ROOT / ".cache/sandbox.lock"
    with lock_path.open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError("another job is active in this checkout") from None
        return _execute(revision, mode, build_timeout, boot_timeout, image, config)


def _execute(revision, mode, build_timeout, boot_timeout, image, config):
    job_id = uuid.uuid4().hex
    directory = JOBS / job_id
    directory.mkdir(parents=True)
    controller = hashlib.sha256()
    for path in sorted([ROOT / "tools/sandbox.py", *(ROOT / "tools/sandbox_support").glob("*.py")]):
        controller.update(path.name.encode() + b"\0" + path.read_bytes())
    state = {"schema_version": 1, "job_id": job_id, "status": "preparing", "revision": revision,
             "controller_sha256": controller.hexdigest(),
             "mode": mode, "image": image, "infrastructure_sha256": config["infrastructure_sha256"],
             "limits": {**runtime.LIMITS, "build_seconds": build_timeout, "boot_seconds": boot_timeout},
             "started_at": time.time(), "artifacts": [], "containers": {}}
    write_state(directory, state)
    print(json.dumps({"event": "started", "job_id": job_id}), file=__import__("sys").stderr, flush=True)
    previous = signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        source = directory / "source.tar"
        if command(["git", "-C", str(ROOT), "archive", "--format=tar", revision],
                   source, limit=runtime.LIMITS["source_bytes"]):
            raise RuntimeError("source export failed or exceeded budget")
        state["source_sha256"] = sha256(source)
        for phase in ("build", "boot"):
            if (directory / "cancel.request").exists():
                state["status"] = "cancelled"
                break
            container = runtime.create(image, job_id, phase, directory)
            if (directory / "cancel.request").exists():
                state["status"] = "cancelled"
                break
            state["status"] = phase + "_running"
            state["containers"][phase] = runtime.inspect(container)
            write_state(directory, state)
            input_path = source if phase == "build" else directory / "kernel.elf"
            worker = ["docker", "exec", "-i", container, "python3", "-I",
                      "/opt/controller/worker.py", phase, revision]
            if phase == "boot":
                worker += [mode, str(boot_timeout)]
            limit = build_timeout if phase == "build" else boot_timeout * (2 if mode in ("block-persist", "block-user") else 1) + 30
            try:
                with input_path.open("rb") as stream:
                    code = command(worker, directory / (phase + ".log"), timeout=limit, stdin=stream)
            except subprocess.TimeoutExpired:
                state["status"] = phase + "_timeout"
                break
            if (directory / "cancel.request").exists():
                state["status"] = "cancelled"
                break
            info = runtime.inspect(container)
            if code:
                state["status"] = "resource_limit" if info and info["State"]["OOMKilled"] else phase + "_failed"
                break
            # Export bounded untrusted bytes; only the separate boot worker interprets the ELF.
            if phase == "build":
                state["artifacts"].append(collect(container, "/work/target/x86_64-unknown-none/release/rustic-os",
                                                  directory / "kernel.elf", 16 * 1024 * 1024))
                for name, maximum in (("sdk-probe.elf", 1024 * 1024), ("app.manifest", 128), ("block-probe.elf", 1024 * 1024), ("block-probe.manifest", 128)):
                    state["artifacts"].append(collect(container, "/work/target/native/" + name,
                                                      directory / name, maximum))
            else:
                sizes = {"result.json": 65536, "image.json": 65536, "serial.log": 1048576,
                         "qemu.log": 1048576, "rustic-os.img": 67108864}
                if mode.startswith("block-"):
                    sizes.update({"block.json": 65536, "blocks.bin": 2048})
                for name, maximum in sizes.items():
                    state["artifacts"].append(collect(container, "/work/out/" + mode + "/" + name,
                                                      directory / name, maximum))
                result = json.loads((directory / "result.json").read_text())
                # Trusted boot worker observes QEMU; never accept candidate build logs as a boot result.
                state["guest_result"] = result
                state["status"] = {"success": "success", "panic": "boot_failed",
                                   "fatal": "boot_failed", "timeout": "boot_timeout"}.get(result["outcome"], "boot_failed")
            runtime.remove_owned(job_id, phase, directory)
    except KeyboardInterrupt:
        (directory / "cancel.request").touch()
        state["status"] = "cancelled"
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        state["status"] = "cancelled" if (directory / "cancel.request").exists() else "executor_error"
        state["error"] = str(error)
    finally:
        signal.signal(signal.SIGTERM, previous)
        cleanup_errors = []
        for phase in ("build", "boot"):
            try:
                runtime.remove_owned(job_id, phase, directory)
            except (RuntimeError, OSError, subprocess.SubprocessError) as error:
                cleanup_errors.append(str(error))
        state["cleanup_errors"] = cleanup_errors
        if cleanup_errors:
            state["status"] = "cleanup_failed"
        state["finished_at"] = time.time()
        state["elapsed_seconds"] = round(state["finished_at"] - state["started_at"], 3)
        state["artifacts"] = [{"path": path.name, "bytes": path.stat().st_size, "sha256": sha256(path)}
                              for path in sorted(directory.iterdir())
                              if path.is_file() and path.name not in ("job.json", "job.json.tmp", "cancel.request")]
        write_state(directory, state)
    return state
