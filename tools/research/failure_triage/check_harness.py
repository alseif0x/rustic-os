# SPDX-License-Identifier: Apache-2.0
"""Offline acceptance of triage guards against actual boot-suite controls."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path

from .diagnose import diagnose_request
from .format import RequestError, read_bounded_bytes, read_json_file, write_json
from .import_report import build_manifest, write_manifest
from .prepare import build_request


def check(boot_directory: Path, output: Path) -> dict:
    output.mkdir(parents=True, exist_ok=False)
    rows = []
    suite_run_id = None
    for mode, expected in (("panic", "panic"), ("hang", "timeout")):
        source = boot_directory / mode
        harness, _, _ = read_json_file(source / "harness.json", limit=48_000, label="harness")
        if suite_run_id is not None and harness["suite_run_id"] != suite_run_id:
            raise RequestError("controls belong to different suite runs")
        suite_run_id = harness["suite_run_id"]
        serial_path = source / "serial.log"
        serial = read_bounded_bytes(serial_path, label="serial")
        if hashlib.sha256(serial).hexdigest() != harness["serial_sha256"]:
            raise RequestError("serial does not match harness")
        if harness["mode"] != mode or harness["expected_outcome"] != expected:
            raise RequestError("unexpected control identity")
        directory = output / mode
        directory.mkdir()
        manifest = build_manifest(
            "boot", source / "result.json", f"{harness['suite_run_id']}/{mode}",
            harness_path=source / "harness.json", output_path=directory / "manifest.json",
        )
        write_manifest(directory / "manifest.json", manifest)
        lines = serial.decode("utf-8").splitlines()
        request = build_request(directory / "manifest.json", [f"{serial_path}:1:{min(40, len(lines))}"])
        write_json(directory / "request.json", request, pretty=False)
        status, reason = diagnose_request(directory / "request.json", directory / "diagnosis.json", live=False)
        report, _, _ = read_json_file(directory / "diagnosis.json", limit=200_000, label="diagnosis")
        if (status != "baseline" or reason is not None or report.get("guard") != "harness_passed"
                or report["hypothesis"]["accepted"] or report["facts"]["outcome"] != expected
                or report["facts"]["harness_passed"] is not True):
            raise RequestError("passed control did not retain its deterministic guard")
        rows.append({"mode": mode, "request_sha256": report["request_sha256"],
                     "harness": harness, "guard": report["guard"], "hypothesis": report["hypothesis"]})
    summary = {"schema_version": 1, "source": "actual boot-suite controls", "live_calls": 0, "controls": rows}
    write_json(output / "summary.json", summary)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--boot-directory", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    check(args.boot_directory, args.output)
    print("Two native boot controls retain deterministic triage guards; no API calls.")


if __name__ == "__main__":
    main()
