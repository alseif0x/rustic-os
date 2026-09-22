# SPDX-License-Identifier: Apache-2.0
"""Import bounded native failure reports into portable triage manifests.

The importer is deliberately a host-only adapter.  It reads one explicitly
selected JSON report, keeps the original value under ``facts.native_report``
and promotes only the fields whose native contract is validated here.  Native
reports are evidence; their arbitrary fields never become instructions or
portable policy.
"""

from __future__ import annotations

import os
from pathlib import Path
import re
from typing import Any

from .format import (
    MAX_REQUEST_BYTES,
    RequestError,
    finite_number,
    read_json_file,
    serialize_json,
    strict_text,
    write_json,
)
from .harness import read_report as read_harness_report
from .schema import validate_manifest


_KIND_RUNNERS = {"boot": "boot", "sandbox": "sandbox", "github-job": "host"}
_BOOT_OUTCOMES = frozenset({"success", "panic", "fatal", "exception", "timeout", "unexpected"})
_SANDBOX_TERMINAL_STATUSES = frozenset(
    {
        "success",
        "build_failed",
        "boot_failed",
        "build_timeout",
        "boot_timeout",
        "resource_limit",
        "executor_error",
        "cleanup_failed",
        "cancelled",
    }
)
_GITHUB_TERMINAL_CONCLUSIONS = frozenset(
    {
        "success",
        "failure",
        "timed_out",
        "cancelled",
        "neutral",
        "skipped",
        "action_required",
        "startup_failure",
        "stale",
    }
)
_HEX64 = re.compile(r"[0-9a-f]{64}\Z")
_HEX_SHA = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")


def _object(value: Any, *, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RequestError(f"{label} must be an object")
    return value


def _integer(value: Any, *, label: str, minimum: int | None = None) -> int:
    if type(value) is not int:
        raise RequestError(f"{label} is not an integer")
    if minimum is not None and value < minimum:
        raise RequestError(f"{label} is below minimum")
    return value


def _boolean(value: Any, *, label: str) -> bool:
    if type(value) is not bool:
        raise RequestError(f"{label} is not a boolean")
    return value


def _text(value: Any, *, label: str) -> str:
    return strict_text(value, label=label)


def _sha256(value: Any, *, label: str) -> str:
    text = _text(value, label=label)
    if _HEX64.fullmatch(text) is None:
        raise RequestError(f"{label} is not a SHA-256 hash")
    return text


def _commit_hash(value: Any, *, label: str) -> str:
    text = _text(value, label=label)
    if _HEX_SHA.fullmatch(text) is None:
        raise RequestError(f"{label} is not a commit hash")
    return text


def _nonnegative_number(value: Any, *, label: str) -> int | float:
    return finite_number(value, label=label, minimum=0.0)


def _reject_alias(input_path: Path, output_path: Path) -> None:
    """Refuse an output path that aliases the selected input inode."""

    try:
        if os.path.samefile(input_path, output_path):
            raise RequestError("input and output must not alias")
    except FileNotFoundError:
        # A new output has no inode to compare.  The lexical check still
        # catches the common same-path case without resolving symlinks.
        pass
    except OSError as error:
        raise RequestError("cannot compare input and output paths") from error

    try:
        input_absolute = os.path.abspath(os.fspath(input_path))
        output_absolute = os.path.abspath(os.fspath(output_path))
    except (TypeError, ValueError) as error:
        raise RequestError("input or output path is invalid") from error
    if input_absolute == output_absolute:
        raise RequestError("input and output must not alias")


def _read_report(input_path: Path, output_path: Path | None) -> tuple[dict[str, Any], str, str]:
    if not isinstance(input_path, Path):
        input_path = Path(input_path)
    if output_path is not None:
        if not isinstance(output_path, Path):
            output_path = Path(output_path)
        _reject_alias(input_path, output_path)

    report, raw, digest = read_json_file(
        input_path,
        limit=MAX_REQUEST_BYTES,
        label="native report",
    )
    # The raw bound is enforced by read_json_file.  Check the compact form as
    # well because the persisted value is the compact form used in the final
    # manifest and JSON spellings can have different lengths.
    try:
        compact_size = len(serialize_json(report))
    except RequestError:
        raise
    if compact_size > MAX_REQUEST_BYTES:
        raise RequestError("native report exceeds compact byte limit")
    report = _object(report, label="native report")
    return report, str(input_path), digest


def _read_harness(
    harness_path: Path,
    native_path: Path,
    output_path: Path | None,
    native_report: dict[str, Any],
    native_digest: str,
) -> tuple[dict[str, Any], str, str]:
    """Read an explicit boot harness attachment and bind it to the result."""

    if not isinstance(harness_path, Path):
        harness_path = Path(harness_path)
    _reject_alias(native_path, harness_path)
    if output_path is not None:
        _reject_alias(harness_path, output_path)
    report, raw, digest = read_harness_report(
        harness_path,
        native_report=native_report,
        native_sha256=native_digest,
    )
    # ``read_json_file`` enforces the raw bound.  Preserve the canonical-size
    # check used for native reports because the complete harness is retained in
    # the portable manifest as well.
    if len(serialize_json(report)) > MAX_REQUEST_BYTES:
        raise RequestError("harness report exceeds compact byte limit")
    return report, str(harness_path), digest


def _source_report(kind: str, path: str, digest: str) -> dict[str, str]:
    _text(path, label="source report path")
    return {"kind": kind, "path": path, "sha256": digest}


def _boot_facts(report: dict[str, Any]) -> dict[str, Any]:
    required = (
        "outcome",
        "returncode",
        "timed_out",
        "build_id",
        "image_sha256",
        "elapsed_seconds",
        "timeout_seconds",
    )
    for field in required:
        if field not in report:
            raise RequestError(f"boot report is missing {field}")

    outcome = _text(report["outcome"], label="boot outcome")
    if outcome not in _BOOT_OUTCOMES:
        raise RequestError("boot outcome is unknown")
    returncode = _integer(report["returncode"], label="boot returncode")
    timed_out = _boolean(report["timed_out"], label="boot timed_out")
    build_id = _text(report["build_id"], label="boot build_id")
    image_sha256 = _sha256(report["image_sha256"], label="boot image_sha256")
    elapsed_seconds = _nonnegative_number(report["elapsed_seconds"], label="boot elapsed_seconds")
    timeout_seconds = _nonnegative_number(report["timeout_seconds"], label="boot timeout_seconds")

    if outcome == "timeout" and timed_out is not True:
        raise RequestError("boot timeout outcome requires timed_out=true")
    if timed_out is True and outcome != "timeout":
        raise RequestError("boot timed_out=true requires timeout outcome")

    facts: dict[str, Any] = {
        "outcome": outcome,
        "returncode": returncode,
        "timed_out": timed_out,
        "build_id": build_id,
        "image_sha256": image_sha256,
        "elapsed_seconds": elapsed_seconds,
        "timeout_seconds": timeout_seconds,
    }
    if "memory_mib" in report:
        facts["memory_mib"] = _integer(report["memory_mib"], label="boot memory_mib", minimum=1)
    return facts


def _sandbox_facts(report: dict[str, Any]) -> dict[str, Any]:
    for field in ("schema_version", "job_id", "revision", "status"):
        if field not in report:
            raise RequestError(f"sandbox report is missing {field}")
    if _integer(report["schema_version"], label="sandbox schema_version") != 1:
        raise RequestError("sandbox schema_version must be 1")
    job_id = _text(report["job_id"], label="sandbox job_id")
    revision = _text(report["revision"], label="sandbox revision")
    status = _text(report["status"], label="sandbox status")
    if status not in _SANDBOX_TERMINAL_STATUSES:
        raise RequestError("sandbox status is not terminal")

    facts: dict[str, Any] = {"status": status, "revision": revision, "job_id": job_id}
    if "mode" in report:
        facts["mode"] = _text(report["mode"], label="sandbox mode")
    if "worker_exit_code" in report:
        facts["worker_exit_code"] = _integer(report["worker_exit_code"], label="sandbox worker_exit_code")
    if "guest_result" in report:
        # Keep the guest observation as a single nested value.  In particular,
        # never turn its returncode/outcome into the outer worker facts.
        facts["guest_result"] = _object(report["guest_result"], label="sandbox guest_result")
    return facts


def _github_facts(report: dict[str, Any]) -> dict[str, Any]:
    required = ("status", "id", "run_id", "name", "conclusion", "head_sha")
    for field in required:
        if field not in report:
            raise RequestError(f"github job report is missing {field}")
    if _text(report["status"], label="github job status") != "completed":
        raise RequestError("github job status must be completed")
    job_id = _integer(report["id"], label="github job id", minimum=1)
    run_id = _integer(report["run_id"], label="github run id", minimum=1)
    name = _text(report["name"], label="github job name")
    conclusion = _text(report["conclusion"], label="github job conclusion")
    if conclusion not in _GITHUB_TERMINAL_CONCLUSIONS:
        raise RequestError("github job conclusion is not terminal")
    head_sha = _commit_hash(report["head_sha"], label="github head_sha")

    facts: dict[str, Any] = {
        "status": conclusion,
        "source_job_id": job_id,
        "source_run_id": run_id,
        "source_name": name,
        "source_head_sha": head_sha,
    }
    if conclusion == "timed_out":
        facts["timed_out"] = True
    return facts


def build_manifest(
    kind: str,
    input_path: Path,
    run_id: str,
    *,
    output_path: Path | None = None,
    harness_path: Path | None = None,
) -> dict[str, Any]:
    """Build one validated portable manifest from one native report.

    ``output_path`` is optional for callers that only need the in-memory
    manifest.  When supplied, input/output aliases are rejected before either
    report is read so an output write can never destroy its source.  A harness
    attachment is accepted only for ``boot`` and is bound to the raw native
    result before its acceptance fields are promoted.
    """

    if kind not in _KIND_RUNNERS:
        raise RequestError("report kind is invalid")
    run_id = _text(run_id, label="manifest run_id")
    report, path_text, digest = _read_report(input_path, output_path)

    if harness_path is not None and kind != "boot":
        raise RequestError("harness attachment is supported only for boot reports")

    if kind == "boot":
        facts = _boot_facts(report)
        if harness_path is not None:
            harness, harness_path_text, harness_digest = _read_harness(
                harness_path,
                Path(input_path),
                output_path,
                report,
                digest,
            )
            # These scalar facts are all independently validated by the
            # harness contract.  Keep the complete association and its source
            # reference namespaced so arbitrary report fields stay inert.
            facts.update(
                {
                    "harness_passed": harness["harness_passed"],
                    "expected_outcome": harness["expected_outcome"],
                    "harness_suite_run_id": harness["suite_run_id"],
                    "harness_mode": harness["mode"],
                    "harness_producer": harness["producer"],
                    "harness_report": harness,
                    "source_harness": _source_report("boot-harness", harness_path_text, harness_digest),
                }
            )
    elif kind == "sandbox":
        facts = _sandbox_facts(report)
    else:
        facts = _github_facts(report)

    # Keep source values intact and keep the original report in exactly one
    # namespaced fact.  Selected scalars above are observations only.
    facts["native_report"] = report
    facts["source_report"] = _source_report(kind, path_text, digest)
    manifest: dict[str, Any] = {
        "schema_version": 1,
        "run_id": run_id,
        "runner": _KIND_RUNNERS[kind],
        "facts": facts,
    }
    validate_manifest(manifest)
    if len(serialize_json(manifest)) > MAX_REQUEST_BYTES:
        raise RequestError("manifest exceeds compact byte limit")
    return manifest


def import_report(
    kind: str,
    input_path: Path,
    run_id: str,
    *,
    output_path: Path | None = None,
    harness_path: Path | None = None,
) -> dict[str, Any]:
    """Compatibility name for :func:`build_manifest`."""

    return build_manifest(kind, input_path, run_id, output_path=output_path, harness_path=harness_path)


def write_manifest(
    output_path: Path,
    manifest: dict[str, Any],
) -> None:
    """Persist a previously built manifest within the same compact bound."""

    validate_manifest(manifest)
    if len(serialize_json(manifest)) > MAX_REQUEST_BYTES:
        raise RequestError("manifest exceeds compact byte limit")
    source = manifest["facts"].get("source_report")
    if isinstance(source, dict) and isinstance(source.get("path"), str):
        _reject_alias(Path(source["path"]), Path(output_path))
    harness_source = manifest["facts"].get("source_harness")
    if isinstance(harness_source, dict) and isinstance(harness_source.get("path"), str):
        _reject_alias(Path(harness_source["path"]), Path(output_path))
    write_json(output_path, manifest, pretty=False, label="manifest")


__all__ = ["build_manifest", "import_report", "write_manifest"]
