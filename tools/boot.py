# SPDX-License-Identifier: Apache-2.0
"""Build and test the R0 UEFI image from the repository root."""
import argparse
from pathlib import Path
import subprocess
import sys
from boot_support.image import DEFAULT_MEMORY_MIB, MAX_MEMORY_MIB, MEMORY_PROFILES, build, memory_supported
from boot_support.runner import run, suite
from boot_support.scenarios import MODES

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["image", "run", "test"])
    parser.add_argument("--mode", choices=MODES, default="ok")
    parser.add_argument("--memory", type=int, default=DEFAULT_MEMORY_MIB,
                        help=f"guest RAM in MiB, 256..{MAX_MEMORY_MIB} in 256 MiB steps (#48); the "
                             "reference `test` suite and the terminal/recovery harnesses remain "
                             f"{DEFAULT_MEMORY_MIB} MiB")
    parser.add_argument("--timeout", type=float, default=30)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 120:
        parser.error("timeout must be between 1 and 120 seconds")
    # The terminal and recovery harnesses own their own QEMU invocation and stay
    # at the reference size. Refuse a profile those paths cannot honor instead of
    # writing a memory_mib that the boot never used. `image.build` enforces the
    # same rule for any caller.
    fixed = args.action == "test" or not memory_supported(args.mode, args.memory)
    if args.memory != DEFAULT_MEMORY_MIB and fixed:
        parser.error(f"--memory {args.memory} is not available for `{args.action} --mode {args.mode}`; "
                     "use tools/memory_profiles_test.py or `run --mode ok|block-*`")
    if args.action == "image":
        print(build(args.mode, args.memory))
    elif args.action == "run":
        result = run(build(args.mode, args.memory), args.timeout, memory=args.memory)
        print(result)
        raise SystemExit({"success": 0, "panic": 1, "fatal": 1, "exception": 1, "timeout": 124}.get(result["outcome"], 2))
    else:
        suite(args.timeout)
        root = Path(__file__).resolve().parent.parent
        for harness in ("v7_read_test.py", "v7_launch_test.py", "v7_corrupt_test.py", "v7_write_test.py",
                        "v7_retention_test.py", "v7_faults_test.py", "v7_admission_test.py",
                        "v7_authority_test.py"):
            subprocess.run(
                [sys.executable, str(root / "tools" / harness)],
                cwd=root,
                check=True,
            )
