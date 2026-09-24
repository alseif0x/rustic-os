# SPDX-License-Identifier: Apache-2.0
"""Migrate three seeded v5 histories to V7 and exercise them in four QEMU boots."""
import subprocess

from boot_support.image import build
from terminal_support.v7_migration import verify
import environment


if __name__ == "__main__":
    subprocess.run(
        ["cargo", "build", "-p", "rustic-volume", "--bin", "rustic-volume", "--locked"],
        cwd=environment.ROOT,
        check=True,
    )
    verify(
        build("terminal-v7"),
        environment.ROOT / "target/debug/rustic-volume",
        environment.ROOT / "artifacts/boot/terminal-v7-migration",
    )
