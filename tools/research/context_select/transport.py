# SPDX-License-Identifier: Apache-2.0
"""Bounded, non-redirecting OpenRouter transport and credential parsing."""

from __future__ import annotations

import os
from pathlib import Path
import socket
import http.client
import urllib.error
import urllib.request

from .format import MAX_RESPONSE_BYTES


DECISIONS_URL = "https://openrouter.ai/api/alpha/decisions"
REQUEST_TIMEOUT_SECONDS = 30
DEFAULT_KEY_FILE = Path("~/.config/rustic-os/openrouter.env").expanduser()


class TransportError(RuntimeError):
    """A live request could not produce a bounded response."""


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):  # type: ignore[override]
        raise TransportError("redirect refused")


def _safe_http_error(error: urllib.error.HTTPError) -> str:
    code = getattr(error, "code", None)
    return f"HTTP {code}" if isinstance(code, int) else "HTTP error"


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
        # Do not include ``reason``: urllib errors can contain a URL, proxy
        # diagnostics, or a credential supplied by an operator.
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
        raw = path.read_bytes()
    except OSError as error:
        raise TransportError("credential file unavailable") from error
    if len(raw) > 8192:
        raise TransportError("credential file is too large")
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
