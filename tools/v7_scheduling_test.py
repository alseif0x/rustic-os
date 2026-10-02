# SPDX-License-Identifier: Apache-2.0
"""Exercise the native V7 scheduler queue and live cancellation contract."""
import subprocess

from boot_support.image import build
from terminal_support.v7_scheduling import verify
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
        environment.ROOT / "artifacts/boot/terminal-v7-scheduling",
    )
