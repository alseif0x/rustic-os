# SPDX-License-Identifier: Apache-2.0
"""Validation for the boot-suite harness association contract.

The boot runner writes this small report beside the native ``result.json`` and
``serial.log`` artifacts.  The report is an association record, not another
source of guest observations: the importer checks the native result bytes and
the identity fields before promoting its acceptance bit.
"""

from __future__ import annotations

import re
from typing import Any

from .format import MAX_REQUEST_BYTES, RequestError, read_json_file, strict_text


SCHEMA_VERSION = 1
PRODUCER = "rustic-boot-suite/v1"

_FIELDS = frozenset(
    {
        "schema_version",
        "producer",
        "suite_run_id",
        "mode",
        "expected_outcome",
        "observed_outcome",
        "reached_fixture",
        "harness_passed",
        "build_id",
        "image_sha256",
        "result_sha256",
        "serial_sha256",
    }
)
_OUTCOMES = frozenset({"success", "panic", "fatal", "exception", "timeout", "unexpected"})
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")


def _object(value: Any, *, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RequestError(f"{label} must be an object")
    return value


def _boolean(value: Any, *, label: str) -> bool:
    if type(value) is not bool:
        raise RequestError(f"{label} is not a boolean")
    return value


def _sha256(value: Any, *, label: str) -> str:
    text = strict_text(value, label=label)
    if _SHA256.fullmatch(text) is None:
        raise RequestError(f"{label} is not a SHA-256 hash")
    return text


def validate_report(
    value: Any,
    *,
    native_report: dict[str, Any] | None = None,
    native_sha256: str | None = None,
) -> dict[str, Any]:
    """Validate one exact harness report.

    When ``native_report`` and ``native_sha256`` are supplied, the association
    is checked against both the parsed result fields and the SHA-256 of the raw
    result file.  Without those arguments this performs only the standalone
    report contract checks, which is useful for producer-side tests.
    """

    report = _object(value, label="harness report")
    if set(report) != _FIELDS:
        raise RequestError("harness report fields are not exact")
    if type(report["schema_version"]) is not int or report["schema_version"] != SCHEMA_VERSION:
        raise RequestError("harness schema_version must be 1")
    if strict_text(report["producer"], label="harness producer") != PRODUCER:
        raise RequestError("harness producer is invalid")
    # The producer uses UUID4 values.  The portable contract treats the value
    # as an opaque non-empty identity so old offline fixtures can retain their
    # own stable IDs without making the importer infer a run from a path.
    strict_text(report["suite_run_id"], label="harness suite_run_id")
    mode = strict_text(report["mode"], label="harness mode")
    if "\x00" in mode or "/" in mode or "\\" in mode:
        raise RequestError("harness mode is invalid")
    expected = strict_text(report["expected_outcome"], label="harness expected_outcome")
    observed = strict_text(report["observed_outcome"], label="harness observed_outcome")
    if expected not in _OUTCOMES or observed not in _OUTCOMES:
        raise RequestError("harness outcome is unknown")
    reached_fixture = _boolean(report["reached_fixture"], label="harness reached_fixture")
    harness_passed = _boolean(report["harness_passed"], label="harness harness_passed")
    if harness_passed is not (observed == expected and reached_fixture):
        raise RequestError("harness_passed contradicts the acceptance predicate")
    build_id = strict_text(report["build_id"], label="harness build_id")
    image_sha256 = _sha256(report["image_sha256"], label="harness image_sha256")
    result_sha256 = _sha256(report["result_sha256"], label="harness result_sha256")
    _sha256(report["serial_sha256"], label="harness serial_sha256")

    if native_report is not None:
        native = _object(native_report, label="native report")
        native_outcome = strict_text(native.get("outcome"), label="boot outcome")
        native_build = strict_text(native.get("build_id"), label="boot build_id")
        native_image = _sha256(native.get("image_sha256"), label="boot image_sha256")
        if observed != native_outcome:
            raise RequestError("harness observed_outcome does not match native report")
        if build_id != native_build:
            raise RequestError("harness build_id does not match native report")
        if image_sha256 != native_image:
            raise RequestError("harness image_sha256 does not match native report")
    if native_sha256 is not None:
        if not isinstance(native_sha256, str) or _SHA256.fullmatch(native_sha256) is None:
            raise RequestError("native report hash is invalid")
        if result_sha256 != native_sha256:
            raise RequestError("harness result_sha256 does not match native report")
    return report


def read_report(
    path,
    *,
    native_report: dict[str, Any] | None = None,
    native_sha256: str | None = None,
) -> tuple[dict[str, Any], bytes, str]:
    """Read and validate a bounded regular JSON harness report."""

    report, raw, digest = read_json_file(path, limit=MAX_REQUEST_BYTES, label="harness report")
    validate_report(report, native_report=native_report, native_sha256=native_sha256)
    return report, raw, digest


__all__ = [
    "PRODUCER",
    "SCHEMA_VERSION",
    "read_report",
    "validate_report",
]
