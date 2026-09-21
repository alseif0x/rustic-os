# SPDX-License-Identifier: Apache-2.0
"""Prepare an immutable failure-triage request from explicit host files."""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Iterable

from .format import MAX_EXCERPTS, MAX_LOGS, MAX_REQUEST_BYTES, RequestError, read_bounded_bytes, read_json_file, serialize_json
from .schema import (
    CHOICE_CRITERIA,
    CLASSIFICATION_INSTRUCTIONS,
    EVIDENCE_CRITERIA,
    EVIDENCE_INSTRUCTIONS_TEMPLATE,
    validate_manifest,
    validate_request,
)
from .format import MODEL, QUERYSET_VERSION


def parse_range_spec(spec: str) -> tuple[str, int, int]:
    """Parse an explicit ``PATH:START:END`` inclusive line range."""

    if not isinstance(spec, str) or not spec or "\x00" in spec:
        raise RequestError("source specification is invalid")
    try:
        source, start_text, end_text = spec.rsplit(":", 2)
        start, end = int(start_text), int(end_text)
    except (TypeError, ValueError) as error:
        raise RequestError("source must be PATH:START:END") from error
    if not source or start < 1 or end < start:
        raise RequestError("source line range is invalid")
    return source, start, end


def _read_range(source: str, start: int, end: int, cache: dict[str, str]) -> str:
    if source not in cache:
        raw = read_bounded_bytes(Path(source), label="excerpt source")
        try:
            cache[source] = raw.decode("utf-8")
        except UnicodeDecodeError as error:
            raise RequestError("excerpt source is not UTF-8") from error
    text = cache[source]
    lines = text.splitlines(keepends=True)
    if end > len(lines):
        raise RequestError("excerpt line range exceeds file")
    selected = "".join(lines[start - 1 : end])
    if not selected:
        raise RequestError("excerpt range is empty")
    return selected


def _question_set(excerpts: list[dict[str, object]]) -> dict[str, dict[str, object]]:
    questions: dict[str, dict[str, object]] = {
        "classification": {
            "type": "choice",
            "instructions": CLASSIFICATION_INSTRUCTIONS,
            "criteria": dict(CHOICE_CRITERIA),
        }
    }
    for excerpt in excerpts:
        excerpt_id = str(excerpt["id"])
        questions[excerpt_id] = {
            "type": "noul",
            "instructions": EVIDENCE_INSTRUCTIONS_TEMPLATE.format(id=excerpt_id),
            "criteria": dict(EVIDENCE_CRITERIA),
        }
    return questions


def build_request(
    manifest_path: Path,
    log_specs: Iterable[str],
    diff_specs: Iterable[str] = (),
) -> dict[str, object]:
    """Read only supplied manifest/ranges and return a validated request."""

    manifest_value, _, _ = read_json_file(Path(manifest_path), limit=MAX_REQUEST_BYTES, label="manifest")
    manifest = validate_manifest(manifest_value)
    logs = list(log_specs)
    diffs = list(diff_specs)
    if len(logs) > MAX_LOGS:
        raise RequestError("log excerpt limit exceeded")
    if len(logs) + len(diffs) > MAX_EXCERPTS:
        raise RequestError("excerpt limit exceeded")
    if not logs and not diffs:
        raise RequestError("at least one --log or --diff is required")

    cache: dict[str, str] = {}
    excerpts: list[dict[str, object]] = []
    seen: set[tuple[str, str, int, int]] = set()
    for kind, specs in (("log", logs), ("diff", diffs)):
        for spec in specs:
            source, start, end = parse_range_spec(spec)
            key = (kind, source, start, end)
            if key in seen:
                raise RequestError("duplicate excerpt range")
            seen.add(key)
            text = _read_range(source, start, end, cache)
            excerpt_id = f"excerpt-{len(excerpts) + 1:03d}"
            excerpts.append(
                {
                    "id": excerpt_id,
                    "path": source,
                    "start": start,
                    "end": end,
                    "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                    "text": text,
                    "kind": kind,
                }
            )

    request = {
        "model": MODEL,
        "state": {
            "manifest": manifest,
            "facts": manifest["facts"],
            "excerpts": excerpts,
            "queryset_version": QUERYSET_VERSION,
        },
        "questions": _question_set(excerpts),
    }
    validate_request(request)
    if len(serialize_json(request)) > MAX_REQUEST_BYTES:
        raise RequestError("request byte limit exceeded")
    return request


__all__ = ["build_request", "parse_range_spec"]
