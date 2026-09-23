# SPDX-License-Identifier: Apache-2.0
"""Build and exercise a read-only V7 artifact workspace in QEMU."""
import subprocess

from boot_support.image import build
from terminal_support.v7_read import verify
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
        environment.ROOT / "artifacts/boot/terminal-v7",
    )
