# SPDX-License-Identifier: Apache-2.0
"""Build immutable JEV requests from explicitly selected repository ranges."""

from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path
from typing import Iterable

from .format import MAX_CANDIDATES, MAX_REQUEST_BYTES, MODEL, RequestError, serialize_json, validate_request


def repository_root() -> Path:
    """Return this checkout's root without consulting the caller's cwd."""

    module_root = Path(__file__).resolve().parents[3]
    try:
        result = subprocess.run(
            ["git", "-C", str(module_root), "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise RequestError("cannot determine repository root") from error
    root = Path(result.stdout.strip()).resolve()
    if not root.is_dir():
        raise RequestError("repository root is invalid")
    return root


def parse_source_spec(spec: str) -> tuple[str, int, int]:
    """Parse ``path:start:end`` with one-based inclusive line numbers."""

    if not isinstance(spec, str) or not spec or "\x00" in spec:
        raise RequestError("source specification is invalid")
    try:
        source, start_text, end_text = spec.rsplit(":", 2)
        start, end = int(start_text), int(end_text)
    except (ValueError, TypeError) as error:
        raise RequestError("source must be path:start:end") from error
    if not source or start < 1 or end < start:
        raise RequestError("source line range is invalid")
    return source, start, end


def _reject_symlink_components(root: Path, relative: Path) -> None:
    current = root
    for part in relative.parts:
        current /= part
        try:
            if current.is_symlink():
                raise RequestError("symlink sources are not allowed")
        except OSError as error:
            raise RequestError("cannot inspect source path") from error


def _resolve_source(root: Path, source: str) -> tuple[Path, str]:
    raw = Path(source)
    if raw.is_absolute() or "\x00" in source:
        raise RequestError("source must be repository-relative")
    # Resolve first to reject ``..`` paths that escape the checkout, while the
    # component walk above rejects both final and intermediate symlinks.
    try:
        relative = raw
        _reject_symlink_components(root, relative)
        path = (root / relative).resolve(strict=True)
        path.relative_to(root)
    except RequestError:
        raise
    except (OSError, ValueError) as error:
        raise RequestError("source is outside the repository or does not exist") from error
    if not path.is_file():
        raise RequestError("source is not a regular file")
    normalized = path.relative_to(root).as_posix()
    if normalized != source.replace("\\", "/"):
        # A normalized spelling is required so request provenance is stable.
        raise RequestError("source path is not normalized")
    try:
        tracked = subprocess.run(
            ["git", "-C", str(root), "--literal-pathspecs", "ls-files", "--error-unmatch", "--", normalized],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise RequestError("source must be a tracked file") from error
    if not tracked.stdout.strip():
        raise RequestError("source must be a tracked file")
    return path, normalized


def _read_range(path: Path, start: int, end: int) -> str:
    try:
        text = path.read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise RequestError("source must be a readable UTF-8 file") from error
    lines = text.splitlines(keepends=True)
    if end > len(lines):
        raise RequestError("source line range exceeds file")
    return "".join(lines[start - 1 : end])


def build_request(task: str, source_specs: Iterable[str], *, root: Path | None = None) -> dict:
    """Read exactly the requested ranges and return a native Decisions request."""

    if not isinstance(task, str) or not task:
        raise RequestError("task must be non-empty UTF-8 text")
    try:
        task.encode("utf-8")
    except UnicodeEncodeError as error:
        raise RequestError("task must be valid UTF-8 text") from error
    specs = list(source_specs)
    if not specs:
        raise RequestError("at least one --source is required")
    if len(specs) > MAX_CANDIDATES:
        raise RequestError("candidate limit exceeded")
    repo = (root or repository_root()).resolve()
    if not repo.is_dir():
        raise RequestError("repository root is invalid")

    candidates = []
    seen_ranges: set[tuple[str, int, int]] = set()
    for index, spec in enumerate(specs, start=1):
        source, start, end = parse_source_spec(spec)
        path, normalized = _resolve_source(repo, source)
        key = (normalized, start, end)
        if key in seen_ranges:
            raise RequestError("duplicate source range")
        seen_ranges.add(key)
        text = _read_range(path, start, end)
        candidates.append(
            {
                "id": f"candidate-{index:03d}",
                "path": normalized,
                "start": start,
                "end": end,
                "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                "text": text,
            }
        )

    questions = {}
    for candidate in candidates:
        candidate_id = candidate["id"]
        questions[candidate_id] = {
            "type": "noul",
            "instructions": (
                f'For candidate ID "{candidate_id}", decide whether this source excerpt '
                "is relevant to the task. Mark true only when the excerpt materially helps "
                "answer or implement the task."
            ),
            "criteria": {
                "true": "The excerpt directly informs the task or an implementation decision.",
                "false": "The excerpt is unrelated or does not materially help with the task.",
            },
        }
    request = {"model": MODEL, "state": {"task": task, "candidates": candidates}, "questions": questions}
    validate_request(request)
    if len(serialize_json(request)) > MAX_REQUEST_BYTES:
        raise RequestError("request byte limit exceeded")
    return request
