# SPDX-License-Identifier: Apache-2.0
"""Accept the existing profile-1 tracked and admission commands on V7."""
import subprocess

from boot_support.image import build
from terminal_support.v7_profile1 import verify
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
        environment.ROOT / "artifacts/boot/terminal-v7-profile1",
    )
