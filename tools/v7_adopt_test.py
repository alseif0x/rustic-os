# SPDX-License-Identifier: Apache-2.0
"""Build terminal-v7 once and exercise storage-sourced file-service adoption."""
import subprocess

from boot_support.image import build
from terminal_support.v7_adopt import verify
import environment


if __name__ == "__main__":
    subprocess.run(
        ["cargo", "build", "-p", "rustic-volume", "--bin", "rustic-volume", "--locked"],
        cwd=environment.ROOT,
        check=True,
    )
    image = build("terminal-v7")
    verify(
        image,
        environment.ROOT / "target/debug/rustic-volume",
        environment.ROOT / "artifacts/boot/terminal-v7-adopt",
    )
