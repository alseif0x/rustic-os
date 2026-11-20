# SPDX-License-Identifier: Apache-2.0
"""Trusted worker installed read-only. Build and boot always use separate containers."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

REFERENCE = Path("/opt/reference")
WORK = Path("/work")
# Built from the reviewed tree at `prepare` (see Dockerfile), never from the
# candidate: the boot runner executes it outside the guest.
VOLUME_TOOL = Path("/opt/volume-tool/rustic-volume")
# Modes whose reviewed runner provisions V7 fixture volumes on the host side.
VOLUME_MODES = ("block-user",)


def receive():
    source = WORK / "src"
    source.mkdir()
    total = 0
    with tarfile.open(fileobj=sys.stdin.buffer, mode="r|") as archive:
        for member in archive:
            parts = Path(member.name).parts
            if member.name.startswith("/") or ".." in parts or ".git" in parts:
                raise RuntimeError("invalid snapshot path")
            if not (member.isdir() or member.isfile()):
                raise RuntimeError("links and special files are not supported")
            total += member.size
            if total > 32 * 1024 * 1024:
                raise RuntimeError("snapshot size limit")
            archive.extract(member, source, filter="data")
    return source


def build(revision, mode):
    source = receive()
    shutil.copytree("/opt/cargo/registry", WORK / "cargo/registry")
    env = os.environ.copy()
    env.update(CARGO_HOME="/work/cargo", CARGO_TARGET_DIR="/work/target",
               CARGO_NET_OFFLINE="true", CARGO_BUILD_JOBS="2", RUSTIC_BUILD_ID=revision[:16], HOME="/work/home")
    (WORK / "home").mkdir()
    sys.path.insert(0, str(REFERENCE / "tools"))
    import application
    env["RUSTIC_APPLICATION_DIRECTORY"] = str(application.build(source, env, offline=True, tasks_acceptance=mode == "terminal-test"))
    command = ["cargo", "build", "-p", "rustic-kernel", "--bin", "rustic-os",
               "--features", "sdk-test", "--target", "x86_64-unknown-none", "--release", "--locked", "--offline", "-vv"]
    return subprocess.call(command, cwd=source, env=env)


def volume_fixtures(mode, environ, work=WORK):
    """Expose only the reviewed volume tool and size scratch space for its fixtures.

    The runner writes three dense 64 MiB V7 regions into its disposable disk,
    which exceeds the 128 MiB /tmp tmpfs; the 1 GiB /work tmpfs holds them.
    A missing tool is refused here instead of falling back to another path.
    """
    environ["RUSTIC_VOLUME_TOOL"] = str(VOLUME_TOOL)
    if mode not in VOLUME_MODES:
        return {}
    try:
        info = os.stat(VOLUME_TOOL, follow_symlinks=False)
    except FileNotFoundError:
        raise RuntimeError(f"reviewed volume tool {VOLUME_TOOL} is missing; rerun prepare") from None
    if not stat.S_ISREG(info.st_mode) or not os.access(VOLUME_TOOL, os.X_OK):
        raise RuntimeError(f"reviewed volume tool {VOLUME_TOOL} is not an executable regular file")
    with VOLUME_TOOL.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    scratch = Path(work) / "tmp"
    scratch.mkdir(mode=0o700)
    environ["TMPDIR"] = str(scratch)
    return {"volume_tool_sha256": digest}


def boot(revision, mode, timeout):
    kernel = WORK / "kernel.elf"
    with kernel.open("wb") as output:
        total = 0
        while chunk := sys.stdin.buffer.read(65536):
            total += len(chunk)
            if total > 16 * 1024 * 1024:
                raise RuntimeError("kernel size limit")
            output.write(chunk)
    fixtures = volume_fixtures(mode, os.environ)
    tempfile.tempdir = os.environ.get("TMPDIR")
    sys.path.insert(0, str(REFERENCE / "tools"))
    from boot_support import image, runner
    image.OUTPUT = WORK / "out"
    path = image.package(kernel, mode, revision[:16], {
        "source_commit": revision, "source_status": "",
        "tasks_acceptance": mode == "terminal-test",
        "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
        **fixtures,
    })
    result = runner.run(path, timeout)
    print(json.dumps(result), flush=True)
    return 0


if __name__ == "__main__":
    action, revision, *args = sys.argv[1:]
    if action == "build":
        raise SystemExit(build(revision, args[0]))
    if action == "boot":
        raise SystemExit(boot(revision, args[0], int(args[1])))
    raise ValueError("unsupported worker action")
