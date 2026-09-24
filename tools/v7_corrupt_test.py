# SPDX-License-Identifier: Apache-2.0
"""Boot the read-only V7 fixture on damaged disposable volumes in QEMU."""
import subprocess

from boot_support.image import build
from terminal_support.v7_corrupt import verify
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
        environment.ROOT / "artifacts/boot/terminal-v7-corrupt",
    )
