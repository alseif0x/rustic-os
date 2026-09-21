# SPDX-License-Identifier: Apache-2.0
"""Offline and optional live diagnosis for a prepared triage request."""

from __future__ import annotations

import json
import re
import time
import urllib.request
from pathlib import Path
from typing import Any

from ..decisions_transport import DEFAULT_KEY_FILE, DECISIONS_URL, TransportError, api_key, post_json
from .baseline import _expected_negative_match, classify
from .format import MAX_REQUEST_BYTES, MODEL, RequestError, load_json_bytes, read_json_file, serialize_json, write_json
from .schema import CLASSIFICATION_OPTIONS, validate_request, validate_response


class ResponseError(ValueError):
    """A provider response does not satisfy the triage Decisions contract."""


def load_request(path: Path) -> tuple[dict[str, Any], bytes, str]:
    """Read and validate a request snapshot without touching excerpt paths."""

    request, raw, request_hash = read_json_file(path, limit=MAX_REQUEST_BYTES, label="request")
    validate_request(request, serialized_size=len(raw))
    return request, raw, request_hash


def _safe_reason(error: Exception) -> str:
    if isinstance(error, TransportError):
        message = str(error)
        if message == "redirect refused" or message.startswith("HTTP "):
            return message
        if re.fullmatch(r"transport [A-Za-z_][A-Za-z0-9_]*", message):
            return message
        if message in {
            "credential unavailable",
            "credential file unavailable",
            "credential file is a symlink",
            "credential file is too large",
            "credential file is not UTF-8",
            "credential format invalid",
            "response byte limit exceeded",
            "response read failed",
        }:
            return message
        return f"transport {type(error).__name__}"
    if isinstance(error, (RequestError, ResponseError, json.JSONDecodeError, UnicodeError, ValueError)):
        return "invalid_response"
    return "live request unavailable"


def _hypothesis_from_answer(answer: dict[str, Any]) -> dict[str, Any]:
    category = answer["choice"]
    confidence = answer.get("confidence")
    accepted = (
        category != "unknown"
        and type(confidence) in (int, float)
        and confidence >= 0.8
    )
    return {"category": category, "confidence": confidence, "accepted": accepted}


def _rank_live_evidence(request: dict[str, Any], answers: dict[str, Any]) -> list[dict[str, Any]]:
    rows: list[tuple[float, int, dict[str, Any]]] = []
    for index, excerpt in enumerate(request["state"]["excerpts"]):
        relevance = answers[excerpt["id"]]["noul"]
        rows.append(
            (
                relevance,
                index,
                {
                    "id": excerpt["id"],
                    "kind": excerpt["kind"],
                    "path": excerpt["path"],
                    "start": excerpt["start"],
                    "end": excerpt["end"],
                    "sha256": excerpt["sha256"],
                    "noul": relevance,
                    "relevance": relevance,
                    "score": relevance,
                },
            )
        )
    rows.sort(key=lambda item: (-item[0], item[1]))
    for rank, (_, _, row) in enumerate(rows, start=1):
        row["rank"] = rank
    return [row for _, _, row in rows]


def _guard_hypothesis(request: dict[str, Any], proposed: dict[str, Any]) -> tuple[dict[str, Any], str | None]:
    facts = request["state"]["facts"]
    if facts.get("harness_passed") is True:
        return {"category": "unknown", "confidence": 0.0, "accepted": False}, "harness_passed"
    if _expected_negative_match(facts) and facts.get("harness_passed") is not False:
        return {"category": "unknown", "confidence": 0.0, "accepted": False}, "expected_negative_outcome"
    if str(facts.get("run_association", "")).casefold() in {"unverified", "unknown", "unavailable"}:
        return {"category": "unknown", "confidence": 0.0, "accepted": False}, "run_association_unverified"
    return proposed, None


def _base_report(request: dict[str, Any], request_hash: str, baseline: dict[str, Any]) -> dict[str, Any]:
    return {
        "request": request,
        "request_sha256": request_hash,
        "facts": request["state"]["facts"],
        "baseline": baseline,
        "hypothesis": dict(baseline["hypothesis"]),
        "evidence": baseline["evidence"],
        "missing_evidence": baseline["missing_evidence"],
        "status": "baseline",
        "fallback": None,
        "usage": None,
        "model": MODEL,
        "provider": None,
        "latency_ms": None,
    }


def diagnose_request(
    request_path: Path,
    output_path: Path,
    *,
    live: bool = False,
    key_file: Path | None = None,
    explicit_key_file: bool = False,
) -> tuple[str, str | None]:
    """Write an advisory report, falling back to baseline on live failure."""

    request, _, request_hash = load_request(Path(request_path))
    baseline = classify(request["state"]["facts"], request["state"]["excerpts"])
    report = _base_report(request, request_hash, baseline)
    status = "baseline"
    reason: str | None = None
    if live:
        started = time.monotonic()
        try:
            key = api_key(key_file=key_file, explicit_key_file=explicit_key_file)
            body = serialize_json(request)
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
                {"classification", *(excerpt["id"] for excerpt in request["state"]["excerpts"])},
            )
            proposed = _hypothesis_from_answer(response["answers"]["classification"])
            guarded, guard = _guard_hypothesis(request, proposed)
            report["model_hypothesis"] = proposed
            report["hypothesis"] = guarded
            report["evidence"] = _rank_live_evidence(request, response["answers"])
            report["model"] = response["model"]
            report["provider"] = response.get("provider")
            report["usage"] = response["usage"]
            report["guard"] = guard
            report["latency_ms"] = round((time.monotonic() - started) * 1000, 3)
            status = "live"
        except (TransportError, RequestError, ResponseError, json.JSONDecodeError, UnicodeError, ValueError) as error:
            reason = _safe_reason(error)
            report["latency_ms"] = round((time.monotonic() - started) * 1000, 3)
            report["status"] = "unavailable"
            report["fallback"] = "baseline"
            report["reason"] = reason
            status = "unavailable"
    report["status"] = status
    if status != "unavailable":
        report.setdefault("guard", baseline.get("guard"))
    write_json(Path(output_path), report, pretty=True, label="report")
    return status, reason


__all__ = ["ResponseError", "diagnose_request", "load_request"]
