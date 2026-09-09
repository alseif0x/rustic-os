# SPDX-License-Identifier: Apache-2.0
"""Local owner CLI for isolated builds. Never expose Docker access to a guest."""
import argparse
import json
import subprocess
from sandbox_support.prepare import prepare
from sandbox_support.jobs import execute, cancel

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("prepare")
    run = commands.add_parser("run")
    run.add_argument("--revision", required=True)
    run.add_argument("--mode", choices=["ok", "panic", "hang", "invalid"], default="ok")
    run.add_argument("--build-timeout", type=int, default=120)
    run.add_argument("--boot-timeout", type=int, default=30)
    stop = commands.add_parser("cancel")
    stop.add_argument("job_id")
    test = commands.add_parser("test")
    test.add_argument("--revision", required=True)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            result = prepare()
        elif args.command == "cancel":
            result = cancel(args.job_id)
        elif args.command == "test":
            from sandbox_support.verification import suite
            result = suite(args.revision)
        else:
            result = execute(args.revision, args.mode, args.build_timeout, args.boot_timeout)
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as error:
        result = {"status": "request_error", "error": str(error)}
    print(json.dumps(result), flush=True)
    raise SystemExit(0 if result["status"] in ("prepared", "success", "cancelled", "cancellation_requested") else 1)
