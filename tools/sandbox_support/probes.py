# SPDX-License-Identifier: Apache-2.0
"""Exercise the configured boundary with synthetic data, not personal files."""
import json
from pathlib import Path
import subprocess
import tempfile
import uuid

from . import runtime
from .prepare import ROOT, STATE

SCRIPT = r'''
import errno, json, os, socket
from pathlib import Path
checks = {}
checks["unprivileged_uid"] = os.getuid() == 1000
status = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines())
checks["no_capabilities"] = int(status["CapEff"].strip(), 16) == 0
checks["no_new_privileges"] = status["NoNewPrivs"].strip() == "1"
checks["seccomp_filter"] = status["Seccomp"].strip() == "2"
checks["no_docker_socket"] = not Path("/var/run/docker.sock").exists()
checks["no_host_checkout"] = not Path(HOST_SENTINEL).exists()
try:
    Path("/opt/root-write-probe").write_text("probe")
    checks["readonly_root"] = False
except OSError as error:
    checks["readonly_root"] = error.errno in (errno.EROFS, errno.EACCES)
try:
    with socket.create_connection(("1.1.1.1", 443), timeout=2):
        checks["no_outbound_network"] = False
except OSError:
    checks["no_outbound_network"] = True
cgroup = Path("/sys/fs/cgroup")
checks["memory_limit"] = (cgroup / "memory.max").read_text().strip() == "2147483648"
checks["swap_disabled"] = (cgroup / "memory.swap.max").read_text().strip() == "0"
quota, period = map(int, (cgroup / "cpu.max").read_text().split())
checks["cpu_limit"] = quota == 2 * period
checks["process_limit"] = (cgroup / "pids.max").read_text().strip() == "128"
for path, limit in (("/work", 1073741824), ("/tmp", 134217728)):
    stat = os.statvfs(path)
    checks[path + "_quota"] = stat.f_blocks * stat.f_frsize == limit
print(json.dumps(checks))
raise SystemExit(0 if all(checks.values()) else 1)
'''


def probe():
    job_id = uuid.uuid4().hex
    directory = ROOT / "artifacts/isolation-probe"
    directory.mkdir(parents=True, exist_ok=True)
    config = json.loads(STATE.read_text())
    with tempfile.NamedTemporaryFile(prefix="rusticos-host-sentinel-") as sentinel:
        try:
            container = runtime.create(config["image"], job_id, "probe", directory)
            info = runtime.inspect(container)
            result = subprocess.run(["docker", "exec", "-i", container, "python3", "-I", "-"],
                                    input="HOST_SENTINEL = " + repr(sentinel.name) + "\n" + SCRIPT,
                                    text=True, capture_output=True, timeout=15)
            if result.returncode:
                raise RuntimeError("isolation probe failed: " + result.stdout + result.stderr)
            checks = json.loads(result.stdout)
            checks["no_host_mounts"] = not info["Mounts"]
            checks["no_devices"] = not info["HostConfig"]["Devices"]
            if not all(checks.values()):
                raise RuntimeError("isolation policy mismatch")
            evidence = {"image": config["image"], "checks": checks}
            (directory / "result.json").write_text(json.dumps(evidence, indent=2) + "\n")
            return evidence
        finally:
            runtime.remove_owned(job_id, "probe", directory)
