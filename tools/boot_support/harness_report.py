# SPDX-License-Identifier: Apache-2.0
"""Write the boot-suite's bounded harness association report.

The runner owns fixture execution and acceptance predicates.  This module only
binds a returned native result to the raw result and serial artifacts after the
execution has completed, so an exception before ``result.json`` exists cannot
be mistaken for a failed or passing guest run.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import stat
import uuid
from typing import Any

SCHEMA_VERSION = 1
PRODUCER = "rustic-boot-suite/v1"
_OUTCOMES = frozenset({"success", "panic", "fatal", "exception", "timeout", "unexpected"})
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")


class HarnessReportError(RuntimeError):
    """The native result or serial artifact cannot be associated safely."""


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise HarnessReportError("duplicate JSON key in boot result")
        result[key] = value
    return result


def _reject_constant(value: str) -> Any:
    raise HarnessReportError(f"non-finite JSON constant {value}")


def _open_regular(path: Path, flags: int) -> int:
    flags |= getattr(os, "O_NONBLOCK", 0)
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        if flags & os.O_CREAT:
            descriptor = os.open(path, flags, 0o600)
        else:
            descriptor = os.open(path, flags)
    except OSError as error:
        raise HarnessReportError(f"cannot open {path}") from error
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise HarnessReportError(f"{path} is not a regular file")
    except HarnessReportError:
        os.close(descriptor)
        raise
    except OSError as error:
        os.close(descriptor)
        raise HarnessReportError(f"cannot inspect {path}") from error
    return descriptor


def _read_regular(path: Path) -> bytes:
    descriptor = _open_regular(path, os.O_RDONLY)
    chunks: list[bytes] = []
    try:
        while True:
            try:
                chunk = os.read(descriptor, 65_536)
            except OSError as error:
                raise HarnessReportError(f"cannot read {path}") from error
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
    finally:
        try:
            os.close(descriptor)
        except OSError:
            pass


def _hash_regular(path: Path) -> str:
    descriptor = _open_regular(path, os.O_RDONLY)
    digest = hashlib.sha256()
    try:
        while True:
            try:
                chunk = os.read(descriptor, 65_536)
            except OSError as error:
                raise HarnessReportError(f"cannot read {path}") from error
            if not chunk:
                return digest.hexdigest()
            digest.update(chunk)
    finally:
        try:
            os.close(descriptor)
        except OSError:
            pass


def _load_result(path: Path) -> tuple[dict[str, Any], bytes, str]:
    raw = _read_regular(path)
    try:
        value = json.loads(
            raw.decode("utf-8"), object_pairs_hook=_pairs, parse_constant=_reject_constant
        )
    except HarnessReportError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError, TypeError, ValueError, RecursionError) as error:
        raise HarnessReportError("boot result is not strict JSON") from error
    if not isinstance(value, dict):
        raise HarnessReportError("boot result must be an object")
    return value, raw, hashlib.sha256(raw).hexdigest()


def _normalise_returned_result(value: Any) -> dict[str, Any]:
    """Apply the same JSON boundary as the native writer before comparison.

    A few host harnesses return tuples inside their result dictionary while
    ``json.dumps`` persists those values as arrays.  Comparing the parsed file
    with that JSON-normalised value verifies the association without rejecting
    an otherwise valid native result solely for its in-memory container type.
    """

    try:
        raw = json.dumps(value, ensure_ascii=False, allow_nan=False).encode("utf-8")
        normalized = json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs, parse_constant=_reject_constant)
    except HarnessReportError:
        raise
    except (UnicodeEncodeError, UnicodeDecodeError, TypeError, ValueError, RecursionError, json.JSONDecodeError) as error:
        raise HarnessReportError("returned boot result is not strict JSON") from error
    if not isinstance(normalized, dict):
        raise HarnessReportError("returned boot result must be an object")
    return normalized


def _strict_text(value: Any, *, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise HarnessReportError(f"{label} is invalid")
    try:
        value.encode("utf-8")
    except UnicodeEncodeError as error:
        raise HarnessReportError(f"{label} is not UTF-8") from error
    return value


def new_suite_run_id() -> str:
    """Return a fresh opaque suite identity."""

    return str(uuid.uuid4())


def _reject_alias(input_path: Path, output_path: Path) -> None:
    """Refuse an output path that aliases an input artifact."""

    try:
        if os.path.samefile(input_path, output_path):
            raise HarnessReportError("harness output must not alias an input")
    except FileNotFoundError:
        pass
    except OSError as error:
        raise HarnessReportError("cannot compare harness input and output paths") from error
    if os.path.abspath(os.fspath(input_path)) == os.path.abspath(os.fspath(output_path)):
        raise HarnessReportError("harness output must not alias an input")


def _sha256(value: Any, *, label: str) -> str:
    text = _strict_text(value, label=label)
    if _SHA256.fullmatch(text) is None:
        raise HarnessReportError(f"{label} is not a SHA-256 hash")
    return text


def _boolean(value: Any, *, label: str) -> bool:
    if type(value) is not bool:
        raise HarnessReportError(f"{label} is not a boolean")
    return value


def _build_report(
    *,
    suite_run_id: str,
    mode: str,
    expected_outcome: str,
    result: dict[str, Any],
    reached_fixture: bool,
    result_sha256: str,
    serial_sha256: str,
) -> dict[str, Any]:
    suite_run_id = _strict_text(suite_run_id, label="harness suite_run_id")
    mode = _strict_text(mode, label="harness mode")
    expected_outcome = _strict_text(expected_outcome, label="harness expected_outcome")
    if expected_outcome not in _OUTCOMES:
        raise HarnessReportError("harness expected_outcome is unknown")
    if not isinstance(result, dict):
        raise HarnessReportError("returned boot result must be an object")
    observed_outcome = _strict_text(result.get("outcome"), label="boot outcome")
    if observed_outcome not in _OUTCOMES:
        raise HarnessReportError("boot outcome is unknown")
    reached_fixture = _boolean(reached_fixture, label="harness reached_fixture")
    result_sha256 = _sha256(result_sha256, label="harness result_sha256")
    serial_sha256 = _sha256(serial_sha256, label="harness serial_sha256")
    build_id = _strict_text(result.get("build_id"), label="boot build_id")
    image_sha256 = _sha256(result.get("image_sha256"), label="boot image_sha256")
    return {
        "schema_version": SCHEMA_VERSION,
        "producer": PRODUCER,
        "suite_run_id": suite_run_id,
        "mode": mode,
        "expected_outcome": expected_outcome,
        "observed_outcome": observed_outcome,
        "reached_fixture": reached_fixture,
        "harness_passed": observed_outcome == expected_outcome and reached_fixture,
        "build_id": build_id,
        "image_sha256": image_sha256,
        "result_sha256": result_sha256,
        "serial_sha256": serial_sha256,
    }


def write_report(
    output_path: Path,
    suite_run_id: str,
    mode: str,
    expected_outcome: str,
    result: dict[str, Any],
    reached_fixture: bool,
    *,
    result_path: Path | None = None,
    serial_path: Path | None = None,
) -> dict[str, Any]:
    """Verify and write one per-mode report, returning its parsed value.

    ``result`` must compare equal to the strictly loaded ``result_path`` value
    before the report can claim an association.  Hashes are over the raw bytes,
    including the native files' original JSON spelling and serial newlines.
    """

    output_path = Path(output_path)
    result_path = Path(result_path or output_path.parent / "result.json")
    serial_path = Path(serial_path or output_path.parent / "serial.log")
    _reject_alias(result_path, output_path)
    _reject_alias(serial_path, output_path)
    loaded, _, result_sha256 = _load_result(result_path)
    if loaded != _normalise_returned_result(result):
        raise HarnessReportError("returned boot result differs from result.json")
    if not isinstance(loaded, dict):
        raise HarnessReportError("boot result must be an object")
    serial_sha256 = _hash_regular(serial_path)
    report = _build_report(
        suite_run_id=suite_run_id,
        mode=mode,
        expected_outcome=expected_outcome,
        result=loaded,
        reached_fixture=reached_fixture,
        result_sha256=result_sha256,
        serial_sha256=serial_sha256,
    )
    _write_report(output_path, report)
    return report


def _write_report(output_path: Path, report: dict[str, Any]) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = _open_regular(output_path, os.O_WRONLY | os.O_CREAT)
    try:
        raw = (json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False) + "\n").encode("utf-8")
        os.ftruncate(descriptor, 0)
        view = memoryview(raw)
        written = 0
        while written < len(view):
            try:
                written += os.write(descriptor, view[written:])
            except OSError as error:
                raise HarnessReportError("cannot write harness report") from error
    except (TypeError, ValueError, UnicodeError, RecursionError) as error:
        raise HarnessReportError("harness report is not strict JSON") from error
    finally:
        try:
            os.close(descriptor)
        except OSError:
            pass


def clear_reports(output: Path) -> None:
    """Remove every stale direct-child ``harness.json`` before a suite run."""

    output = Path(output)
    if not output.exists():
        return
    for path in output.glob("*/harness.json"):
        path.unlink()


__all__ = [
    "HarnessReportError",
    "PRODUCER",
    "SCHEMA_VERSION",
    "clear_reports",
    "new_suite_run_id",
    "write_report",
]
