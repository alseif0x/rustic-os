# SPDX-License-Identifier: Apache-2.0
"""Verify the default manual terminal on fresh disposable V7 media."""
import json
import subprocess
from boot_support.image import build, package
from terminal_support.v7_manual import verify
import environment


if __name__ == "__main__":
    subprocess.run(["cargo", "build", "-p", "rustic-volume", "--locked"],
                   cwd=environment.ROOT, check=True)
    image = build("terminal-init")
    identity = json.loads((image.parent / "image.json").read_text())
    provenance = {key: value for key, value in identity.items()
                  if key not in ("mode", "build_id", "kernel_sha256", "image_sha256", "environment")}
    mount = package(image.parent / "kernel.elf", "terminal", identity["build_id"], provenance)
    verify(image, mount, environment.ROOT / "target/debug/rustic-volume")
