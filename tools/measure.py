# SPDX-License-Identifier: Apache-2.0
"""Collect native R0 measurements or compare compatible, complete reports."""
import argparse
import json
from pathlib import Path
import subprocess
from measurement.model import compare


def report(path):
    path = Path(path)
    if path.stat().st_size > 5 * 1024 * 1024:
        raise ValueError("measurement report exceeds 5 MiB")
    return json.loads(path.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    for name in ("run", "verify"):
        command = sub.add_parser(name)
        command.add_argument("--output", required=True, help="new output directory; existing paths are refused")
        command.add_argument("--host-label", required=True)
        command.add_argument("--samples", type=int, default=5)
        if name == "run":
            command.add_argument("--inject-delay-ticks", type=int, choices=(0, 100), default=0)
    command = sub.add_parser("compare")
    command.add_argument("baseline")
    command.add_argument("candidate")
    command.add_argument("--output")
    args = parser.parse_args()
    try:
        if args.action == "compare":
            result = compare(report(args.baseline), report(args.candidate))
            if args.output:
                with Path(args.output).open("x") as stream:
                    json.dump(result, stream, indent=2, allow_nan=False)
                    stream.write("\n")
            code = {"pass": 0, "regression": 2, "incomparable": 3}[result["status"]]
        else:
            from measurement.series import execute
            result = execute(args.output, args.host_label, args.samples, args.action == "verify",
                             getattr(args, "inject_delay_ticks", 0))
            code = 0 if result["status"] == "success" else 1
        print(json.dumps(result, allow_nan=False))
        return code
    except (OSError, RuntimeError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(json.dumps({"status": "request_error", "error": str(error)}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
