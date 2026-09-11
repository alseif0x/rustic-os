# SPDX-License-Identifier: Apache-2.0
"""Identify the measured environment; keep dynamic host load separate."""
import os
import platform
import re
import struct
import subprocess
import tempfile
from pathlib import Path
import environment
from .model import fingerprint
from .cgroups import configuration as cgroup_configuration


def read(path, default="unavailable"):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return default


def filesystem(path):
    path = str(Path(path).resolve())
    matches = [(len(parts[1]), parts[2]) for line in read("/proc/mounts", "").splitlines()
               if len(parts := line.split()) >= 3
               and (path == parts[1] or path.startswith(parts[1].rstrip("/") + "/"))]
    return max(matches)[1] if matches else "unavailable"


def configuration(root, output, label):
    cpu = re.search(r"(?m)^model name\s*:\s*(.+)$", read("/proc/cpuinfo"))
    sources = [*Path(__file__).parent.glob("*.py"), root / "tools/measure.py"]
    sources += [root / "tools/terminal_support" / n for n in
                ("machine.py", "connection.py", "cases.py", "authority_cases.py",
                 "management_cases.py", "oracle.py", "oracle_admission.py", "provision.py")]
    sources += [root / "tools/boot_support" / n for n in
                ("runner.py", "scenarios.py", "image.py", "process_evidence.py",
                 "ipc_evidence.py", "sdk_evidence.py", "block_evidence.py", "block_user_evidence.py")]
    sources += [root / "tools" / n for n in ("environment.py", "application.py")]
    return {
        "workload": "r0-owner-control-v1",
        "harness": fingerprint({str(p.relative_to(root)): environment.digest(p) for p in sorted(sources)}),
        "reference": environment.CONFIG, "guest_mib": 256, "guest_cpus": 1,
        "data_bytes": 4 * 1024 ** 3, "cache": "writeback", "network": "none",
        "sample_protocol": {"warmups": 1, "admitted_hold_ticks": 200, "timeout_seconds": 60},
        "host": {"label": label, "system": platform.system(), "release": platform.release(),
                 "architecture": platform.machine(), "cpu_model": cpu[1] if cpu else "unavailable",
                 "logical_cpus": os.cpu_count(), "affinity": sorted(os.sched_getaffinity(0)),
                 "memory": read("/proc/meminfo").splitlines()[0],
                 "boot_identity_sha256": fingerprint(read("/proc/sys/kernel/random/boot_id")),
                 "cgroup": cgroup_configuration(),
                 "output_filesystem": filesystem(output), "temporary_filesystem": filesystem(tempfile.gettempdir()),
                 "python": platform.python_version()},
        "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
    }


def load_bytes(kernel):
    data = kernel.read_bytes()
    if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected ELF64 little-endian kernel")
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    if size != 56 or count == 0 or count > 32 or offset + size * count > len(data):
        raise ValueError("invalid program-header table")
    ranges = []
    for index in range(count):
        kind, _, _, address, _, filesz, memsz, _ = struct.unpack_from("<IIQQQQQQ", data, offset + index * size)
        if kind == 1:
            if filesz > memsz or not memsz:
                raise ValueError("invalid load segment")
            ranges.append((address // 4096, (address + memsz + 4095) // 4096))
    if not ranges:
        raise ValueError("missing load segments")
    total, end = 0, 0
    for start, stop in sorted(ranges):
        total += max(0, stop - max(start, end))
        end = max(end, stop)
    return total * 4096


def observation():
    return {"load_average": list(os.getloadavg())}
