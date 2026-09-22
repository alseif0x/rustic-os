# SPDX-License-Identifier: Apache-2.0
"""Wire format and validation for the host-only JEV context pilot.

This module deliberately knows nothing about repository discovery or HTTP.  It
validates the immutable request snapshot before a caller can send it anywhere.
"""

from __future__ import annotations

import hashlib
import json
import math
import re
from pathlib import Path
from typing import Any


MODEL = "typesafe/jev-1.13"
MAX_CANDIDATES = 32
MAX_REQUEST_BYTES = 48_000
MAX_RESPONSE_BYTES = 1_048_576


class RequestError(ValueError):
    """A request is malformed or exceeds the local pilot bounds."""


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise RequestError("duplicate JSON key")
        result[key] = value
    return result


def _reject_constant(value: str) -> Any:
    raise RequestError(f"non-finite JSON constant {value}")


def load_json_bytes(raw: bytes) -> Any:
    """Decode strict UTF-8 JSON without accepting duplicate or non-finite data."""

    try:
        return json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_pairs,
            parse_constant=_reject_constant,
        )
    except RequestError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError, TypeError) as error:
        raise RequestError("invalid JSON request") from error


def serialize_json(value: Any) -> bytes:
    """Use one compact UTF-8 representation for requests and size checks."""

    try:
        return json.dumps(
            value,
            ensure_ascii=False,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
    except (TypeError, ValueError, UnicodeError, RecursionError) as error:
        raise RequestError("request is not JSON serializable") from error


def _is_text(value: Any) -> bool:
    return isinstance(value, str)


def _is_int(value: Any) -> bool:
    return type(value) is int


def _is_finite_number(value: Any) -> bool:
    return type(value) in (int, float) and math.isfinite(float(value))


def _exact_keys(value: Any, expected: set[str], label: str) -> None:
    if not isinstance(value, dict) or set(value) != expected:
        raise RequestError(f"{label} fields are not exact")


def _relative_path(path: Any) -> None:
    if not _is_text(path) or not path or "\x00" in path:
        raise RequestError("candidate path is invalid")
    # Requests are produced with POSIX repository-relative paths.  Refusing
    # alternate spellings keeps provenance stable when a request is replayed.
    if path.startswith("/") or "\\" in path:
        raise RequestError("candidate path is not repository-relative")
    parts = path.split("/")
    if any(part in ("", ".", "..") for part in parts):
        raise RequestError("candidate path is not normalized")


def validate_request(request: Any, *, serialized_size: int | None = None) -> dict[str, Any]:
    """Validate a prepared native Decisions request and return it unchanged."""

    if not isinstance(request, dict):
        raise RequestError("request must be an object")
    _exact_keys(request, {"model", "state", "questions"}, "request")
    if request["model"] != MODEL:
        raise RequestError("unsupported model")

    state = request["state"]
    if not isinstance(state, dict):
        raise RequestError("state must be an object")
    _exact_keys(state, {"task", "candidates"}, "state")
    task = state["task"]
    if not _is_text(task) or not task:
        raise RequestError("task must be non-empty UTF-8 text")
    candidates = state["candidates"]
    if not isinstance(candidates, list) or not candidates:
        raise RequestError("at least one candidate is required")
    if len(candidates) > MAX_CANDIDATES:
        raise RequestError("candidate limit exceeded")

    questions = request["questions"]
    if not isinstance(questions, dict):
        raise RequestError("questions must be an object")
    candidate_ids: list[str] = []
    seen_ids: set[str] = set()
    for candidate in candidates:
        if not isinstance(candidate, dict):
            raise RequestError("candidate must be an object")
        _exact_keys(candidate, {"id", "path", "start", "end", "sha256", "text"}, "candidate")
        candidate_id = candidate["id"]
        if not _is_text(candidate_id) or not candidate_id or candidate_id in seen_ids:
            raise RequestError("candidate IDs must be unique non-empty text")
        seen_ids.add(candidate_id)
        candidate_ids.append(candidate_id)
        _relative_path(candidate["path"])
        if not _is_int(candidate["start"]) or not _is_int(candidate["end"]):
            raise RequestError("candidate line numbers must be integers")
        if candidate["start"] < 1 or candidate["end"] < candidate["start"]:
            raise RequestError("candidate line range is invalid")
        digest = candidate["sha256"]
        if not _is_text(digest) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise RequestError("candidate hash is invalid")
        text = candidate["text"]
        if not _is_text(text):
            raise RequestError("candidate text must be UTF-8 text")
        if hashlib.sha256(text.encode("utf-8")).hexdigest() != digest:
            raise RequestError("candidate text hash mismatch")

    if set(questions) != seen_ids:
        raise RequestError("questions do not exactly match candidates")
    for candidate_id in candidate_ids:
        question = questions[candidate_id]
        if not isinstance(question, dict):
            raise RequestError("question must be an object")
        _exact_keys(question, {"type", "instructions", "criteria"}, "question")
        if question["type"] != "noul":
            raise RequestError("questions must use noul")
        instructions = question["instructions"]
        if not _is_text(instructions) or candidate_id not in instructions:
            raise RequestError("question must identify its candidate")
        criteria = question["criteria"]
        if not isinstance(criteria, dict):
            raise RequestError("question criteria must be an object")
        _exact_keys(criteria, {"true", "false"}, "question criteria")
        if not _is_text(criteria["true"]) or not _is_text(criteria["false"]):
            raise RequestError("question criteria must be text")

    serialized = serialize_json(request)
    size = len(serialized) if serialized_size is None else serialized_size
    if size > MAX_REQUEST_BYTES:
        raise RequestError("request byte limit exceeded")
    return request


def load_request(path: Path) -> tuple[dict[str, Any], bytes, str]:
    """Read, hash, size-check and validate a request snapshot from disk."""

    try:
        with path.open("rb") as stream:
            raw = stream.read(MAX_REQUEST_BYTES + 1)
    except OSError as error:
        raise RequestError("cannot read request") from error
    if len(raw) > MAX_REQUEST_BYTES:
        raise RequestError("request byte limit exceeded")
    request = load_json_bytes(raw)
    validate_request(request, serialized_size=len(raw))
    return request, raw, hashlib.sha256(raw).hexdigest()


def write_json(path: Path, value: Any, *, pretty: bool = False) -> None:
    """Write a UTF-8 JSON artifact, creating its parent directory if needed."""

    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        if pretty:
            raw = json.dumps(value, ensure_ascii=False, indent=2, sort_keys=False, allow_nan=False).encode("utf-8")
        else:
            raw = serialize_json(value)
        path.write_bytes(raw)
    except (OSError, TypeError, ValueError, UnicodeError, RecursionError) as error:
        raise RequestError("cannot write JSON artifact") from error
