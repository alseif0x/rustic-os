# SPDX-License-Identifier: Apache-2.0
"""Start two separately built utility versions from V7 storage on one kernel image in QEMU."""
import subprocess

from boot_support.image import build
from terminal_support import v7_launch
import environment


if __name__ == "__main__":
    subprocess.run(
        ["cargo", "build", "-p", "rustic-volume", "--bin", "rustic-volume", "--locked"],
        cwd=environment.ROOT,
        check=True,
    )
    image = build("terminal-v7")
    tool = environment.ROOT / "target/debug/rustic-volume"
    v7_launch.verify(image, tool, environment.ROOT / "artifacts/boot/terminal-v7-launch")
