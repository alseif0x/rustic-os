# SPDX-License-Identifier: Apache-2.0
"""Strict wire validation and bounded host-file access for failure triage."""

from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import stat
from typing import Any


MODEL = "typesafe/jev-1.13"
QUERYSET_VERSION = "failure-triage-v1"
MAX_LOGS = 32
MAX_EXCERPTS = 32
MAX_REQUEST_BYTES = 48_000
MAX_INPUT_FILE_BYTES = 4 * 1024 * 1024


class RequestError(ValueError):
    """A local manifest, request, excerpt or output is invalid."""


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
    """Decode strict UTF-8 JSON without duplicate or non-finite values."""

    try:
        value = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_pairs,
            parse_constant=_reject_constant,
        )
    except RequestError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError, RecursionError, TypeError) as error:
        raise RequestError("invalid JSON") from error
    try:
        _validate_json_text(value)
    except RecursionError as error:
        raise RequestError("JSON nesting is too deep") from error
    return value


def _validate_json_text(value: Any) -> None:
    """Reject lone UTF-16 surrogates accepted by Python's JSON decoder."""

    if isinstance(value, bool) or value is None:
        return
    if isinstance(value, (int, float)):
        if isinstance(value, float) and not math.isfinite(value):
            raise RequestError("JSON number is not finite")
        return
    if isinstance(value, str):
        try:
            value.encode("utf-8")
        except UnicodeEncodeError as error:
            raise RequestError("JSON text is not UTF-8") from error
    elif isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str):
                raise RequestError("JSON object key is not text")
            _validate_json_text(key)
            _validate_json_text(child)
    elif isinstance(value, list):
        for child in value:
            _validate_json_text(child)


def serialize_json(value: Any) -> bytes:
    """Serialize one compact finite UTF-8 JSON representation."""

    try:
        raw = json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    except (TypeError, ValueError, UnicodeError, RecursionError) as error:
        raise RequestError("value is not strict JSON") from error
    return raw


def _absolute_path(path: Path) -> Path:
    return path if path.is_absolute() else Path.cwd() / path


def reject_symlink_components(path: Path, *, message: str = "file is a symlink") -> None:
    """Reject existing symlink components without resolving the path."""

    absolute = _absolute_path(path)
    current = Path(absolute.anchor)
    for part in absolute.parts[1:]:
        current /= part
        try:
            if current.is_symlink():
                raise RequestError(message)
        except OSError as error:
            raise RequestError("cannot inspect file path") from error


def read_bounded_bytes(path: Path, *, limit: int = MAX_INPUT_FILE_BYTES, label: str = "file") -> bytes:
    """Read a regular UTF-8 input without following a symlink or truncating."""

    if not isinstance(path, Path):
        path = Path(path)
    reject_symlink_components(path, message=f"{label} is a symlink")
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0)
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise RequestError(f"cannot read {label}") from error
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            raise RequestError(f"{label} is not a regular file")
        chunks: list[bytes] = []
        total = 0
        while total <= limit:
            chunk = os.read(descriptor, min(65_536, limit + 1 - total))
            if not chunk:
                break
            chunks.append(chunk)
            total += len(chunk)
            if total > limit:
                raise RequestError(f"{label} is too large")
    except RequestError:
        raise
    except (OSError, ValueError) as error:
        raise RequestError(f"cannot read {label}") from error
    finally:
        try:
            os.close(descriptor)
        except OSError:
            pass
    return b"".join(chunks)


def read_json_file(path: Path, *, limit: int, label: str) -> tuple[Any, bytes, str]:
    """Read, parse, size-check and hash a strict JSON artifact."""

    raw = read_bounded_bytes(path, limit=limit, label=label)
    value = load_json_bytes(raw)
    return value, raw, hashlib.sha256(raw).hexdigest()


def write_bounded_bytes(path: Path, raw: bytes, *, label: str = "output") -> None:
    """Write an artifact to a regular path without following symlinks."""

    if not isinstance(path, Path):
        path = Path(path)
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise RequestError(f"cannot create {label} directory") from error
    reject_symlink_components(path, message=f"{label} is a symlink")
    # Open without truncating first.  A final fstat keeps FIFOs/devices from
    # blocking or being overwritten before we have established a regular file.
    flags = os.O_WRONLY | os.O_CREAT | getattr(os, "O_NONBLOCK", 0)
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags, 0o600)
    except OSError as error:
        raise RequestError(f"cannot write {label}") from error
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            raise RequestError(f"{label} is not a regular file")
        os.ftruncate(descriptor, 0)
        view = memoryview(raw)
        written = 0
        while written < len(view):
            written += os.write(descriptor, view[written:])
    except (OSError, ValueError) as error:
        raise RequestError(f"cannot write {label}") from error
    finally:
        try:
            os.close(descriptor)
        except OSError:
            pass


def write_json(path: Path, value: Any, *, pretty: bool = True, label: str = "output") -> None:
    """Write a strict JSON artifact."""

    try:
        if pretty:
            raw = json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False).encode("utf-8")
        else:
            raw = serialize_json(value)
    except (TypeError, ValueError, UnicodeError, RecursionError) as error:
        raise RequestError(f"{label} is not strict JSON") from error
    write_bounded_bytes(path, raw, label=label)


def exact_keys(value: Any, expected: set[str], label: str) -> None:
    if not isinstance(value, dict) or set(value) != expected:
        raise RequestError(f"{label} fields are not exact")


def strict_text(value: Any, *, label: str, nonempty: bool = True) -> str:
    if not isinstance(value, str) or (nonempty and not value):
        raise RequestError(f"{label} is invalid")
    try:
        value.encode("utf-8")
    except UnicodeEncodeError as error:
        raise RequestError(f"{label} is not UTF-8") from error
    return value


def finite_number(value: Any, *, label: str, minimum: float | None = None, maximum: float | None = None) -> int | float:
    if isinstance(value, bool) or type(value) not in (int, float):
        raise RequestError(f"{label} is not a number")
    try:
        if not math.isfinite(float(value)):
            raise RequestError(f"{label} is not finite")
    except (OverflowError, ValueError) as error:
        raise RequestError(f"{label} is not finite") from error
    if minimum is not None and value < minimum:
        raise RequestError(f"{label} is below minimum")
    if maximum is not None and value > maximum:
        raise RequestError(f"{label} is above maximum")
    return value


__all__ = [
    "MAX_EXCERPTS",
    "MAX_INPUT_FILE_BYTES",
    "MAX_LOGS",
    "MAX_REQUEST_BYTES",
    "MODEL",
    "QUERYSET_VERSION",
    "RequestError",
    "exact_keys",
    "finite_number",
    "load_json_bytes",
    "read_bounded_bytes",
    "read_json_file",
    "reject_symlink_components",
    "serialize_json",
    "strict_text",
    "write_bounded_bytes",
    "write_json",
]
