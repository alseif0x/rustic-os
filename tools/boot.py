# SPDX-License-Identifier: Apache-2.0
"""Build and test the R0 UEFI image from the repository root."""
import argparse
from boot_support.image import build
from boot_support.runner import run, suite
from boot_support.scenarios import MODES

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["image", "run", "test"])
    parser.add_argument("--mode", choices=MODES, default="ok")
    parser.add_argument("--timeout", type=float, default=30)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 120:
        parser.error("timeout must be between 1 and 120 seconds")
    if args.action == "image":
        print(build(args.mode))
    elif args.action == "run":
        result = run(build(args.mode), args.timeout)
        print(result)
        raise SystemExit({"success": 0, "panic": 1, "fatal": 1, "exception": 1, "timeout": 124}.get(result["outcome"], 2))
    else:
        suite(args.timeout)
