# SPDX-License-Identifier: Apache-2.0
"""Verify existing owner maintenance and stall commands on V7."""
import subprocess

from boot_support.image import build
from terminal_support.v7_owner import verify
import environment


if __name__ == "__main__":
    subprocess.run(["cargo", "build", "-p", "rustic-volume", "--locked"],
                   cwd=environment.ROOT, check=True)
    verify(build("terminal-v7"), environment.ROOT / "target/debug/rustic-volume")
