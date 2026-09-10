# SPDX-License-Identifier: Apache-2.0
"""Resolve supported cgroup v2 scopes and record inherited resource limits."""
import re
from pathlib import Path, PurePosixPath
from .model import fingerprint


def _cgroup_read(path, *, optional=False):
    try:
        return path.read_text().strip()
    except FileNotFoundError:
        if optional:
            return None
    except (OSError, UnicodeError):
        pass
    # Scope and mount paths can contain usernames. Do not expose them in errors.
    raise ValueError("cgroup provenance is unreadable or incomplete")


def _mount_path(value):
    value = re.sub(r"\\(040|011|012|134)", lambda m: chr(int(m[1], 8)), value)
    path = PurePosixPath(value)
    if not value.startswith("/") or ".." in path.parts or str(path) != value:
        raise ValueError("unsupported cgroup mount path")
    return path


def _cgroup_mount(membership, mountinfo):
    entries = membership.splitlines()
    if len(entries) != 1 or not entries[0].startswith("0::/"):
        raise ValueError("measurement requires a unified cgroup v2 hierarchy")
    member = entries[0][3:]
    if ".." in PurePosixPath(member).parts or str(PurePosixPath(member)) != member:
        raise ValueError("cgroup membership is outside the visible hierarchy")
    roots = []
    for line in mountinfo.splitlines():
        before, separator, after = line.partition(" - ")
        fields, filesystem_fields = before.split(), after.split()
        if not separator or len(fields) < 6 or len(filesystem_fields) < 3:
            raise ValueError("invalid mount information for cgroup provenance")
        if filesystem_fields[0] == "cgroup2" and fields[3] == "/":
            roots.append(Path(str(_mount_path(fields[4]))))
    if len(roots) != 1:
        raise ValueError("cgroup provenance requires one complete root mount")
    return member, roots[0]


def _cgroup_limit(value, name):
    if name == "cpu.max":
        parts = value.split()
        if (len(parts) == 2 and re.fullmatch(r"max|[1-9][0-9]*", parts[0])
                and re.fullmatch(r"[1-9][0-9]*", parts[1])):
            return " ".join(parts)
    elif name == "memory.max":
        if re.fullmatch(r"max|0|[1-9][0-9]*", value):
            return value
    elif re.fullmatch(r"[0-9]+(?:-[0-9]+)?(?:,[0-9]+(?:-[0-9]+)?)*", value):
        previous = -1
        for span in value.split(","):
            bounds = [int(item) for item in span.split("-")]
            start, end = bounds[0], bounds[-1]
            if start <= previous or start > end:
                break
            previous = end
        else:
            return value
    raise ValueError("invalid cgroup limit: " + name)


def _cgroup_level(path, depth):
    kind = _cgroup_read(path / "cgroup.type", optional=depth == 0)
    if depth == 0 and kind is not None:
        raise ValueError("cgroup namespace hides enforceable ancestor limits")
    if depth > 0 and kind != "domain":
        raise ValueError("measurement requires domain cgroups; threaded scopes are unsupported")
    controllers = sorted(_cgroup_read(path / "cgroup.controllers").split())
    if depth == 0 and not {"cpu", "memory", "cpuset"}.issubset(controllers):
        raise ValueError("full root CPU, memory and cpuset visibility is required")
    result = {"depth": depth, "controllers": controllers}
    for controller, name in (("cpu", "cpu.max"), ("memory", "memory.max"),
                             ("cpuset", "cpuset.cpus.effective"), ("cpuset", "cpuset.mems.effective")):
        value = _cgroup_read(path / name, optional=True)
        root_limit = depth == 0 and controller in ("cpu", "memory")
        if root_limit and value is not None:
            # A cgroup namespace can label its delegated subtree "/". The real
            # hierarchy root has no cpu.max/memory.max; accepting this view would
            # silently omit limits imposed above the visible namespace root.
            raise ValueError("cgroup namespace hides enforceable ancestor limits")
        expected = controller in controllers and not root_limit
        if expected != (value is not None):
            raise ValueError("cgroup controller interfaces are incomplete or unsupported")
        result[name.replace(".", "_")] = _cgroup_limit(value, name) if value is not None else None
    return result


def configuration(proc=Path("/proc")):
    """Record every visible root-to-process limit, never infer available capacity.

    Unified domain hierarchies only. Reject mounts/namespaces hiding ancestors;
    shared parent quotas are recorded even when the leaf itself is unlimited.
    See https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html.
    """
    membership = _cgroup_read(proc / "self/cgroup")
    mountinfo = _cgroup_read(proc / "self/mountinfo")
    member, mount = _cgroup_mount(membership, mountinfo)
    levels = [_cgroup_level(mount, 0)]
    current = mount
    for depth, component in enumerate(PurePosixPath(member).parts[1:], 1):
        current /= component
        levels.append(_cgroup_level(current, depth))
    if (membership != _cgroup_read(proc / "self/cgroup")
            or mountinfo != _cgroup_read(proc / "self/mountinfo")):
        raise ValueError("cgroup membership or mounts changed during collection")
    return {"version": 2, "membership_sha256": fingerprint(member), "ancestors": levels}
