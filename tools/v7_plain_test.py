# SPDX-License-Identifier: Apache-2.0
"""Exercise ordinary terminal file operations on a disposable V7 volume."""
import subprocess

from boot_support.image import build
from terminal_support.v7_plain import verify
import environment


if __name__ == "__main__":
    subprocess.run(
        ["cargo", "build", "-p", "rustic-volume", "--bin", "rustic-volume", "--locked"],
        cwd=environment.ROOT,
        check=True,
    )
    verify(build("terminal-v7"), environment.ROOT / "target/debug/rustic-volume")
