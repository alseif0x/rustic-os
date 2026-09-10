# SPDX-License-Identifier: Apache-2.0
"""Launch the native RusticOS serial terminal with its own persistent image."""
import argparse
from pathlib import Path
from boot_support.image import build
from terminal_support.machine import disk, machine
import environment

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--initialize", action="store_true", help="create a NEW dedicated disk; never overwrite an existing one")
    args = parser.parse_args()
    directory = environment.ROOT / "artifacts/terminal"
    image = build("terminal-init" if args.initialize else "terminal")
    with disk(directory / "data.raw", args.initialize) as data:
        print("Starting native RusticOS. Type exit for a clean stop. QEMU emergency exit: Ctrl-A X.", flush=True)
        with machine(image, data, "stdio", directory / "qemu.log") as process:
            code = process.wait()
    return 0 if code == 33 else 1

if __name__ == "__main__":
    raise SystemExit(main())
