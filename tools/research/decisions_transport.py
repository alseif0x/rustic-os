# SPDX-License-Identifier: Apache-2.0
"""Bounded, non-redirecting transport shared by host Decisions pilots.

This module contains only host-side credential parsing and one-shot HTTP.  It
never interprets a credential file as shell input, follows redirects, retries a
request, or exposes response/URL details in an exception message.
"""

from __future__ import annotations

import http.client
import os
from pathlib import Path
import socket
import stat
import urllib.error
import urllib.request


DECISIONS_URL = "https://openrouter.ai/api/alpha/decisions"
REQUEST_TIMEOUT_SECONDS = 30
MAX_RESPONSE_BYTES = 1_048_576
MAX_CREDENTIAL_BYTES = 8192
DEFAULT_KEY_FILE = Path("~/.config/rustic-os/openrouter.env").expanduser()


class TransportError(RuntimeError):
    """A live request or bounded credential read could not complete."""


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):  # type: ignore[override]
        raise TransportError("redirect refused")


def _safe_http_error(error: urllib.error.HTTPError) -> str:
    code = getattr(error, "code", None)
    return f"HTTP {code}" if isinstance(code, int) else "HTTP error"


def _absolute_path(path: Path) -> Path:
    """Return an absolute lexical path without resolving symlinks."""

    return path if path.is_absolute() else Path.cwd() / path


def _reject_symlink_components(path: Path) -> None:
    """Reject every existing symlink component before opening ``path``."""

    absolute = _absolute_path(path)
    current = Path(absolute.anchor)
    for part in absolute.parts[1:]:
        current /= part
        try:
            if current.is_symlink():
                raise TransportError("credential file is a symlink")
        except OSError as error:
            raise TransportError("credential file unavailable") from error


def _read_credential_bytes(path: Path) -> bytes:
    """Read a small regular file without following a symlink."""

    _reject_symlink_components(path)
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0)
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise TransportError("credential file unavailable") from error
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            raise TransportError("credential file unavailable")
        data = os.read(descriptor, MAX_CREDENTIAL_BYTES + 1)
    except (OSError, ValueError) as error:
        if isinstance(error, TransportError):
            raise
        raise TransportError("credential file unavailable") from error
    finally:
        try:
            os.close(descriptor)
        except OSError:
            pass
    if len(data) > MAX_CREDENTIAL_BYTES:
        raise TransportError("credential file is too large")
    return data


def post_json(request: urllib.request.Request) -> bytes:
    """POST exactly once through an opener that cannot follow redirects."""

    opener = urllib.request.build_opener(_NoRedirect)
    try:
        response = opener.open(request, timeout=REQUEST_TIMEOUT_SECONDS)
    except TransportError:
        raise
    except urllib.error.HTTPError as error:
        raise TransportError(_safe_http_error(error)) from error
    except (urllib.error.URLError, TimeoutError, socket.timeout, ConnectionError, OSError, http.client.HTTPException) as error:
        raise TransportError(f"transport {type(error).__name__}") from error
    try:
        data = response.read(MAX_RESPONSE_BYTES + 1)
    except (OSError, ValueError, TimeoutError, socket.timeout, http.client.HTTPException) as error:
        raise TransportError("response read failed") from error
    finally:
        try:
            response.close()
        except OSError:
            pass
    if len(data) > MAX_RESPONSE_BYTES:
        raise TransportError("response byte limit exceeded")
    return data


def _parse_key_file(path: Path) -> str:
    try:
        raw = _read_credential_bytes(path)
    except TransportError:
        raise
    try:
        content = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise TransportError("credential file is not UTF-8") from error
    found: str | None = None
    for line in content.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if "=" in stripped:
            name, value = stripped.split("=", 1)
            if name.strip() != "OPENROUTER_API_KEY":
                continue
            value = value.strip()
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
                value = value[1:-1]
            found = value
            break
        # A selected key file may contain one raw token.  It is still data;
        # no shell grammar is interpreted.
        found = stripped
        break
    if not found:
        raise TransportError("credential unavailable")
    if any(character.isspace() for character in found):
        raise TransportError("credential format invalid")
    return found


def api_key(*, key_file: Path | None = None, explicit_key_file: bool = False) -> str:
    """Select an environment key or parse a dotenv-like file as plain data."""

    if explicit_key_file:
        return _parse_key_file(key_file or DEFAULT_KEY_FILE)
    environment = os.environ.get("OPENROUTER_API_KEY")
    if environment:
        if any(character.isspace() for character in environment):
            raise TransportError("credential format invalid")
        return environment
    return _parse_key_file(key_file or DEFAULT_KEY_FILE)


__all__ = [
    "DEFAULT_KEY_FILE",
    "DECISIONS_URL",
    "MAX_RESPONSE_BYTES",
    "REQUEST_TIMEOUT_SECONDS",
    "TransportError",
    "_NoRedirect",
    "_parse_key_file",
    "api_key",
    "post_json",
]
