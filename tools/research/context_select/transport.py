# SPDX-License-Identifier: Apache-2.0
"""Compatibility exports for the shared host Decisions transport."""

from __future__ import annotations

# Keep ``transport.urllib.request`` available for existing focused tests and
# callers that patch the opener.  The shared implementation imports the same
# module object, so patching this compatibility view affects both pilots.
import urllib
import urllib.error
import urllib.request

from ..decisions_transport import (
    DEFAULT_KEY_FILE,
    DECISIONS_URL,
    MAX_RESPONSE_BYTES,
    REQUEST_TIMEOUT_SECONDS,
    TransportError,
    _NoRedirect,
    _parse_key_file,
    api_key,
    post_json,
)

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
