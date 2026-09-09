# SPDX-License-Identifier: Apache-2.0
"""Fixed Docker policy. No caller-supplied mounts, devices, flags, or commands."""
import json
import subprocess
from .process import command

LIMITS = {"cpus": 2, "memory_bytes": 2147483648, "swap_bytes": 0,
          "work_bytes": 1073741824, "tmp_bytes": 134217728, "pids": 128,
          "network": "none", "attempts": 1, "source_bytes": 33554432}


def name(job_id, phase):
    return "rusticos-" + job_id + "-" + phase


def create(image, job_id, phase, directory):
    container = name(job_id, phase)
    args = [
        "docker", "create", "--name", container, "--label", "rusticos.job=" + job_id,
        "--label", "rusticos.phase=" + phase, "--read-only", "--network", "none",
        "--cap-drop", "ALL", "--security-opt", "no-new-privileges=true",
        "--user", "1000:1000", "--cpus", "2", "--memory", "2g", "--memory-swap", "2g",
        "--pids-limit", "128", "--log-driver", "none", "--ulimit", "core=0",
        "--ulimit", "nofile=256:256", "--ulimit", "fsize=268435456:268435456",
        "--tmpfs", "/work:rw,exec,nosuid,nodev,size=1073741824,uid=1000,gid=1000,mode=0700",
        "--tmpfs", "/tmp:rw,nosuid,nodev,size=134217728,uid=1000,gid=1000,mode=0700",
        image,
    ]
    if command(args, directory / (phase + "-create.log")):
        raise RuntimeError("container creation failed")
    if command(["docker", "start", container], directory / (phase + "-start.log")):
        raise RuntimeError("container start failed")
    return container


def inspect(container):
    result = subprocess.run(["docker", "inspect", container], capture_output=True, text=True, timeout=15)
    if result.returncode:
        if "no such object" in result.stderr.lower() or "no such container" in result.stderr.lower():
            return None
        raise RuntimeError("cannot inspect container: " + result.stderr[:1000])
    return json.loads(result.stdout)[0]


def remove_owned(job_id, phase, directory):
    container = name(job_id, phase)
    info = inspect(container)
    if info is None:
        return
    if info["Config"].get("Labels", {}).get("rusticos.job") != job_id:
        raise RuntimeError("refusing to remove container with different owner label")
    if command(["docker", "rm", "-f", container], directory / (phase + "-cleanup.log")):
        if inspect(container) is not None:
            raise RuntimeError("container cleanup failed")
