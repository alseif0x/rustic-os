# SPDX-License-Identifier: Apache-2.0
"""Trusted fixed-name boot evidence exporter; never walks candidate directories."""
import io
import json
import os
from pathlib import Path
import re
import stat
import sys
import tarfile

MAX_BUNDLE = 8 * 1024 * 1024
MAX_PAYLOAD = MAX_BUNDLE - 256 * 1024  # Fixed headers, padding and capture report.
REPORT = "boot-failure-capture.json"
BASE = {"image.json": 65536, "result.json": 65536,
        "serial.log": 1048576, "qemu.log": 1048576}
SESSIONS = ("initial", "reboot",
            *(name + ending for name in ("data", "receipt", "metadata", "header", "final_flush")
              for ending in ("-fault", "-reboot")),
            "admitted_data", "admitted_data-reboot", "admitted_final_flush",
            "admitted_final_flush-reboot", "legacy", "upgrade", "operations-initial", "operations-reboot",
            *("operations-" + name + ending for name in ("data", "receipt", "metadata", "header", "final_flush")
              for ending in ("-fault", "-reboot")),
            *("scheduling_authority_" + name + ending
              for name in ("human_edit", "read_write", "inspect_all", "scope_subject", "revoked_cancel")
              for ending in ("", "-reboot")))


def session_files(name):
    if name not in SESSIONS:
        raise ValueError("unknown recovery session")
    return {f"failure-{name}.json": 65536,
            f"failure-{name}.files.bin": 89088,
            f"failure-{name}.last-sector.bin": 512,
            f"{name}.commands.jsonl": 2 * 1024 * 1024,
            f"{name}.serial.log": 1048576, f"{name}.qemu.log": 1048576}


def base_files(mode):
    """Share fixed boot-output budgets with normal and failure collection."""
    if re.fullmatch(r"[a-z]+(?:-[a-z]+)*", mode) is None:
        raise ValueError("invalid boot mode")
    result = dict(BASE)
    if mode == "recovery-test":
        # Expanded recovery reports exceed 64 KiB; retain a fixed 128 KiB ceiling.
        result.update({"result.json": 128 * 1024, "recovery.json": 128 * 1024})
    return result


def allowed_files(mode):
    result = {**base_files(mode), REPORT: 65536}
    if mode == "recovery-test":
        for name in SESSIONS:
            result.update(session_files(name))
    return result


def _read(directory, name, maximum):
    descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    with os.fdopen(descriptor, "rb") as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
            raise ValueError("evidence must be a dedicated regular file")
        if not 0 <= before.st_size <= maximum:
            raise ValueError("evidence exceeds its size limit")
        data = source.read(maximum + 1)
        after = os.fstat(source.fileno())
        if (len(data) != before.st_size or after.st_size != before.st_size
                or after.st_mtime_ns != before.st_mtime_ns):
            raise ValueError("evidence changed during capture")
        return data


def _unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate failure metadata key")
        result[key] = value
    return result


def export(root, mode, output):
    """Stream at most 8 MiB; missing files and rejected captures stay explicit."""
    allowed_files(mode)
    report = {"schema_version": 1, "mode": mode, "captured": [], "missing": [], "errors": []}
    used = 0
    directory = None
    with tarfile.open(fileobj=output, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
        def add(name, payload):
            nonlocal used
            if used + len(payload) > MAX_PAYLOAD:
                raise ValueError("failure evidence bundle budget exceeded")
            member = tarfile.TarInfo(name)
            member.size = len(payload)
            member.mode = 0o600
            archive.addfile(member, io.BytesIO(payload))
            used += len(payload)
            report["captured"].append(name)

        def capture(name, maximum, optional=False):
            try:
                payload = _read(directory, name, maximum)
                add(name, payload)
                return payload
            except FileNotFoundError:
                if not optional:
                    report["missing"].append(name)
            except (OSError, ValueError) as error:
                report["errors"].append({"path": name, "error": str(error)[:200]})
            return None

        try:
            # /work is trusted tmpfs; reject symlink replacements of out, mode or files.
            parent = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                directory = os.open(mode, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            finally:
                os.close(parent)
            for name, maximum in base_files(mode).items():
                capture(name, maximum, optional=name in ("result.json", "recovery.json"))
            if mode == "recovery-test":
                for session in SESSIONS:
                    files = session_files(session)
                    name = f"failure-{session}.json"
                    payload = capture(name, files.pop(name), optional=True)
                    if payload is None:
                        continue
                    try:
                        failure = json.loads(payload, object_pairs_hook=_unique)
                        if (not isinstance(failure, dict) or failure.get("verified") is not False
                                or failure.get("session") != session):
                            raise ValueError("failure metadata does not identify this session")
                    except (UnicodeError, ValueError, RecursionError) as error:
                        report["errors"].append({"path": name, "error": str(error)[:200]})
                        continue
                    for name, maximum in files.items():
                        capture(name, maximum)
        except (OSError, ValueError) as error:
            report["errors"].append({"path": "boot output", "error": str(error)[:200]})
        finally:
            if directory is not None:
                os.close(directory)
        report["status"] = "partial" if report["missing"] or report["errors"] else "captured"
        payload = json.dumps(report, indent=2).encode() + b"\n"
        if len(payload) > 65536:
            raise ValueError("capture report exceeds budget")
        member = tarfile.TarInfo(REPORT)
        member.size = len(payload)
        member.mode = 0o600
        archive.addfile(member, io.BytesIO(payload))


if __name__ == "__main__":
    export(Path("/work/out"), sys.argv[1], sys.stdout.buffer)
