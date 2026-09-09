# SPDX-License-Identifier: Apache-2.0
"""Trusted worker installed read-only. Build and boot always use separate containers."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

REFERENCE = Path("/opt/reference")
WORK = Path("/work")


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


def build(revision):
    source = receive()
    shutil.copytree("/opt/cargo/registry", WORK / "cargo/registry")
    env = os.environ.copy()
    env.update(CARGO_HOME="/work/cargo", CARGO_TARGET_DIR="/work/target",
               CARGO_NET_OFFLINE="true", CARGO_BUILD_JOBS="2", RUSTIC_BUILD_ID=revision[:16], HOME="/work/home")
    (WORK / "home").mkdir()
    sys.path.insert(0, str(REFERENCE / "tools"))
    import application
    env["RUSTIC_APPLICATION_DIRECTORY"] = str(application.build(source, env, offline=True))
    command = ["cargo", "build", "-p", "rustic-kernel", "--bin", "rustic-os",
               "--features", "sdk-test", "--target", "x86_64-unknown-none", "--release", "--locked", "--offline", "-vv"]
    return subprocess.call(command, cwd=source, env=env)


def boot(revision, mode, timeout):
    kernel = WORK / "kernel.elf"
    with kernel.open("wb") as output:
        total = 0
        while chunk := sys.stdin.buffer.read(65536):
            total += len(chunk)
            if total > 16 * 1024 * 1024:
                raise RuntimeError("kernel size limit")
            output.write(chunk)
    sys.path.insert(0, str(REFERENCE / "tools"))
    from boot_support import image, runner
    image.OUTPUT = WORK / "out"
    path = image.package(kernel, mode, revision[:16], {
        "source_commit": revision, "source_status": "",
        "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
    })
    result = runner.run(path, timeout)
    print(json.dumps(result), flush=True)
    return 0


if __name__ == "__main__":
    action, revision, *args = sys.argv[1:]
    if action == "build":
        raise SystemExit(build(revision))
    if action == "boot":
        raise SystemExit(boot(revision, args[0], int(args[1])))
    raise ValueError("unsupported worker action")
