# SPDX-License-Identifier: Apache-2.0
"""Deterministic and optional live ranking for a prepared request."""

from __future__ import annotations

import json
import math
import re
import time
import urllib.request
from pathlib import Path
from typing import Any

from .format import RequestError, load_json_bytes, load_request
from .transport import DEFAULT_KEY_FILE, DECISIONS_URL, TransportError, api_key, post_json


class ResponseError(ValueError):
    """A response does not satisfy the native Decisions contract."""


def _tokens(value: str) -> set[str]:
    return {token for token in re.findall(r"\w+", value.casefold(), flags=re.UNICODE) if token}


def _candidate_copy(candidate: dict[str, Any]) -> dict[str, Any]:
    # Keep all source metadata and text in every output row.  A shallow copy is
    # enough because these fields are scalar strings and integers.
    return dict(candidate)


def _lexical_rows(request: dict[str, Any]) -> list[dict[str, Any]]:
    task_tokens = _tokens(request["state"]["task"])
    rows: list[tuple[float, int, dict[str, Any]]] = []
    for index, candidate in enumerate(request["state"]["candidates"]):
        candidate_tokens = _tokens(candidate["text"])
        overlap = len(task_tokens & candidate_tokens)
        score = overlap / len(task_tokens) if task_tokens else 0.0
        row = _candidate_copy(candidate)
        row["baseline_score"] = score
        rows.append((score, index, row))
    rows.sort(key=lambda item: (-item[0], item[1]))
    for rank, (_, _, row) in enumerate(rows, start=1):
        row["rank"] = rank
    return [row for _, _, row in rows]


def _number(value: Any, *, label: str, integer: bool = False) -> int | float:
    if isinstance(value, bool) or type(value) not in ((int,) if integer else (int, float)):
        raise ResponseError(f"{label} has invalid type")
    try:
        finite = math.isfinite(float(value))
    except (OverflowError, ValueError):
        finite = False
    if not finite:
        raise ResponseError(f"{label} is not finite")
    return value


def _response_text(value: Any, *, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ResponseError(f"{label} is invalid")
    try:
        value.encode("utf-8")
    except UnicodeEncodeError as error:
        raise ResponseError(f"{label} is not UTF-8") from error
    return value


def validate_response(response: Any, candidate_ids: set[str]) -> dict[str, Any]:
    """Validate the strict subset of the native response used by this pilot."""

    if not isinstance(response, dict):
        raise ResponseError("response is not an object")
    allowed = {"answers", "id", "model", "provider", "usage"}
    if set(response) - allowed or not {"answers", "model", "usage"} <= set(response):
        raise ResponseError("response fields are incomplete or extra")
    _response_text(response["model"], label="response model")
    if "id" in response:
        _response_text(response["id"], label="response id")
    if "provider" in response:
        _response_text(response["provider"], label="response provider")

    answers = response["answers"]
    if not isinstance(answers, dict) or set(answers) != candidate_ids:
        raise ResponseError("answers are incomplete or contain extras")
    for candidate_id, answer in answers.items():
        if not isinstance(answer, dict) or set(answer) != {"type", "noul"}:
            raise ResponseError(f"answer {candidate_id} is malformed")
        if answer["type"] != "noul" or isinstance(answer["noul"], bool) or type(answer["noul"]) not in (int, float):
            raise ResponseError(f"answer {candidate_id} is not a noul number")
        value = answer["noul"]
        try:
            finite = math.isfinite(float(value))
        except (OverflowError, ValueError):
            finite = False
        if not finite or not 0.0 <= value <= 1.0:
            raise ResponseError(f"answer {candidate_id} is out of range")

    usage = response["usage"]
    if not isinstance(usage, dict) or set(usage) - {"input_tokens", "output_tokens", "cost"}:
        raise ResponseError("usage fields are malformed")
    if not {"input_tokens", "output_tokens"} <= set(usage):
        raise ResponseError("usage fields are incomplete")
    for name in ("input_tokens", "output_tokens"):
        value = _number(usage[name], label=name, integer=True)
        if value < 0:
            raise ResponseError(f"{name} is negative")
    if "cost" in usage:
        value = _number(usage["cost"], label="cost")
        if value < 0:
            raise ResponseError("cost is negative")
    return response


def _live_rows(request: dict[str, Any], response: dict[str, Any]) -> list[dict[str, Any]]:
    rows: list[tuple[float, int, dict[str, Any]]] = []
    answers = response["answers"]
    for index, candidate in enumerate(request["state"]["candidates"]):
        row = _candidate_copy(candidate)
        row["noul"] = answers[candidate["id"]]["noul"]
        rows.append((row["noul"], index, row))
    rows.sort(key=lambda item: (-item[0], item[1]))
    for rank, (_, _, row) in enumerate(rows, start=1):
        row["rank"] = rank
    return [row for _, _, row in rows]


def _report_base(request: dict[str, Any], request_hash: str, ranking: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "task": request["state"]["task"],
        "request": request,
        "request_sha256": request_hash,
        "requested_model": request["model"],
        "model": request["model"],
        "provider": None,
        "usage": None,
        "latency_ms": None,
        "ranking": ranking,
    }


def _safe_reason(error: Exception) -> str:
    """Keep transport diagnostics useful without serializing operator data."""

    if isinstance(error, TransportError):
        message = str(error)
        if message == "redirect refused" or message.startswith("HTTP "):
            return message
        if re.fullmatch(r"transport [A-Za-z_][A-Za-z0-9_]*", message):
            return message
        if message in {
            "credential unavailable",
            "credential file unavailable",
            "credential file is too large",
            "credential file is not UTF-8",
            "credential format invalid",
            "response byte limit exceeded",
            "response read failed",
        }:
            return message
        return f"transport {type(error).__name__}"
    if isinstance(error, (ResponseError, RequestError)):
        return "invalid_response"
    return "live request unavailable"


def rank_request(
    request_path: Path,
    output_path: Path,
    *,
    live: bool = False,
    key_file: Path | None = None,
    explicit_key_file: bool = False,
) -> tuple[str, str | None]:
    """Rank one request, saving a report even when the live path is unavailable."""

    request, _, request_hash = load_request(request_path)
    baseline = _lexical_rows(request)
    report = _report_base(request, request_hash, baseline)
    status = "baseline"
    reason: str | None = None
    if live:
        started = time.monotonic()
        try:
            key = api_key(key_file=key_file, explicit_key_file=explicit_key_file)
            body = json.dumps(request, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
            http_request = urllib.request.Request(
                DECISIONS_URL,
                data=body,
                method="POST",
                headers={
                    "Authorization": f"Bearer {key}",
                    "Content-Type": "application/json",
                    "Accept": "application/json",
                },
            )
            response = validate_response(
                load_json_bytes(post_json(http_request)),
                {candidate["id"] for candidate in request["state"]["candidates"]},
            )
            report["model"] = response["model"]
            report["provider"] = response.get("provider")
            report["usage"] = response["usage"]
            report["latency_ms"] = round((time.monotonic() - started) * 1000, 3)
            report["ranking"] = _live_rows(request, response)
            status = "live"
        except (TransportError, ResponseError, RequestError, json.JSONDecodeError, UnicodeError, ValueError) as error:
            # The reason is always one of our static/sanitized messages.  Do
            # not serialize exception text originating in response bodies.
            reason = _safe_reason(error)
            report["latency_ms"] = round((time.monotonic() - started) * 1000, 3)
            report["status"] = "unavailable"
            report["fallback"] = "baseline"
            report["reason"] = reason
            status = "unavailable"
    report.setdefault("status", status)
    if status != "unavailable":
        report["fallback"] = None
    try:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_bytes(json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False).encode("utf-8"))
    except (OSError, TypeError, ValueError, UnicodeError) as error:
        raise RequestError("cannot write ranking report") from error
    return status, reason
