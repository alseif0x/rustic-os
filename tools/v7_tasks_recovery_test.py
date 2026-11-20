# SPDX-License-Identifier: Apache-2.0
"""Verify TASKS_OWNER lost-reply recovery in two native V7 boots."""
import json
import subprocess

from boot_support.image import build, package
from terminal_support.v7_tasks_recovery import verify
import environment


if __name__ == "__main__":
    source = build("terminal-test")
    source_metadata = json.loads((source.parent / "image.json").read_text())
    if source_metadata.get("mode") != "terminal-test" or source_metadata.get("tasks_acceptance") is not True:
        raise RuntimeError("terminal-test did not produce a tasks-acceptance image")
    provenance = dict(source_metadata)
    provenance["mode"] = "terminal-v7"
    image = package(source.parent / "kernel.elf", "terminal-v7",
                    source_metadata["build_id"], provenance)
    packaged = json.loads((image.parent / "image.json").read_text())
    if packaged.get("mode") != "terminal-v7" or packaged.get("tasks_acceptance") is not True:
        raise RuntimeError("terminal-v7 packaging lost the tasks-acceptance build metadata")

    subprocess.run(["cargo", "build", "-p", "rustic-volume", "--bin", "rustic-volume", "--locked"],
                   cwd=environment.ROOT, check=True)
    verify(image, environment.ROOT / "target/debug/rustic-volume")
