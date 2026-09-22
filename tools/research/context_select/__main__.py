# SPDX-License-Identifier: Apache-2.0
"""Command-line entry point for the bounded context-ranking pilot."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys

from .format import RequestError, write_json
from .prepare import build_request
from .rank import DEFAULT_KEY_FILE, rank_request


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="python3 -m tools.research.context_select")
    commands = parser.add_subparsers(dest="command", required=True)

    prepare = commands.add_parser("prepare", help="prepare an explicit source-range request")
    prepare.add_argument("--task", required=True)
    prepare.add_argument("--source", action="append", required=True, metavar="PATH:START:END")
    prepare.add_argument("--output", required=True, type=Path)

    rank = commands.add_parser("rank", help="rank a prepared request")
    rank.add_argument("--request", required=True, type=Path)
    rank.add_argument("--output", required=True, type=Path)
    rank.add_argument("--live", action="store_true", help="make one native Decisions request")
    rank.add_argument("--key-file", type=Path, default=None, metavar="FILE")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "prepare":
            request = build_request(args.task, args.source)
            write_json(args.output, request)
            print(f"status=prepared output={args.output}")
            return 0

        key_file = args.key_file if args.key_file is not None else DEFAULT_KEY_FILE
        status, reason = rank_request(
            args.request,
            args.output,
            live=args.live,
            key_file=key_file,
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
