# SPDX-License-Identifier: Apache-2.0
"""Command-line entry point for host-only failure triage."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys

from .diagnose import diagnose_request
from .format import RequestError, write_json
from .import_report import build_manifest, write_manifest
from .prepare import build_request


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="python3 -m tools.research.failure_triage")
    commands = parser.add_subparsers(dest="command", required=True)

    prepare = commands.add_parser("prepare", help="prepare a bounded explicit-source triage request")
    prepare.add_argument("--manifest", required=True, type=Path)
    prepare.add_argument("--log", action="append", default=[], metavar="PATH:START:END")
    prepare.add_argument("--diff", action="append", default=[], metavar="PATH:START:END")
    prepare.add_argument("--output", required=True, type=Path)

    diagnose = commands.add_parser("diagnose", help="diagnose a prepared triage request")
    diagnose.add_argument("--request", required=True, type=Path)
    diagnose.add_argument("--output", required=True, type=Path)
    diagnose.add_argument("--live", action="store_true", help="make one native Decisions request")
    diagnose.add_argument("--key-file", type=Path, default=None, metavar="FILE")

    import_report = commands.add_parser(
        "import-report",
        help="import one bounded native report into a portable triage manifest",
    )
    import_report.add_argument("--kind", choices=("boot", "sandbox", "github-job"), required=True)
    import_report.add_argument("--input", required=True, type=Path)
    import_report.add_argument("--harness", type=Path, default=None,
                               help="optional boot-suite harness.json (only with --kind boot)")
    import_report.add_argument("--run-id", required=True)
    import_report.add_argument("--output", required=True, type=Path)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "prepare":
            request = build_request(args.manifest, args.log, args.diff)
            # Keep the persisted snapshot within the same compact 48 KiB wire
            # bound that was validated before writing it.
            write_json(args.output, request, pretty=False, label="request")
            print(f"status=prepared output={args.output}")
            return 0

        if args.command == "import-report":
            manifest = build_manifest(
                args.kind,
                args.input,
                args.run_id,
                output_path=args.output,
                harness_path=args.harness,
            )
            write_manifest(args.output, manifest)
            print(f"status=imported output={args.output}")
            return 0

        status, reason = diagnose_request(
            args.request,
            args.output,
            live=args.live,
            key_file=args.key_file,
            explicit_key_file=args.key_file is not None,
        )
        if reason:
            print(f"status={status} output={args.output} reason={reason}")
        else:
            print(f"status={status} output={args.output}")
        return 2 if status == "unavailable" else 0
    except (RequestError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
