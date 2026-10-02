# SPDX-License-Identifier: Apache-2.0
"""Verify existing native consumers on a freshly provisioned V7 volume."""
import subprocess

from boot_support.image import build
from terminal_support.v7_consumers import verify
import environment


if __name__ == "__main__":
    subprocess.run(["cargo", "build", "-p", "rustic-volume", "--locked"],
                   cwd=environment.ROOT, check=True)
    verify(build("terminal-v7"), environment.ROOT / "target/debug/rustic-volume")
