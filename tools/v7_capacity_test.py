# SPDX-License-Identifier: Apache-2.0
"""Exhaust the storage of a nearly full V7 volume across two QEMU boots."""
import subprocess

from boot_support.image import build
from terminal_support.v7_capacity import verify
import environment


if __name__ == "__main__":
    # The fill mounts the growing 64 MiB image once per `add7`; an optimized
    # host tool keeps that to seconds instead of minutes. The guest is unchanged.
    subprocess.run(
        ["cargo", "build", "--release", "-p", "rustic-volume", "--bin", "rustic-volume", "--locked"],
        cwd=environment.ROOT,
        check=True,
    )
    verify(
        build("terminal-v7"),
        environment.ROOT / "target/release/rustic-volume",
        environment.ROOT / "artifacts/boot/terminal-v7-capacity",
    )
