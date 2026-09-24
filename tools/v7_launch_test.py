# SPDX-License-Identifier: Apache-2.0
"""Start two separately built utility versions from V7 storage on one kernel image in QEMU.

The same built image then boots once more on a single migrated V7 volume that
holds both versions, and the owner rolls back from the newer to the older pair.
"""
import subprocess

from boot_support.image import build
from terminal_support import v7_launch, v7_rollback
import environment


if __name__ == "__main__":
    subprocess.run(
        ["cargo", "build", "-p", "rustic-volume", "--bin", "rustic-volume", "--locked"],
        cwd=environment.ROOT,
        check=True,
    )
    image = build("terminal-v7")
    tool = environment.ROOT / "target/debug/rustic-volume"
    launch = v7_launch.verify(image, tool, environment.ROOT / "artifacts/boot/terminal-v7-launch")
    rollback = v7_rollback.verify(image, tool, environment.ROOT / "artifacts/boot/terminal-v7-rollback")
    if (launch["image_sha256"], launch["kernel_sha256"]) != (rollback["image_sha256"], rollback["kernel_sha256"]):
        raise SystemExit("the launch and rollback boots used different kernel images")
