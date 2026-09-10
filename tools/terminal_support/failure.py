# SPDX-License-Identifier: Apache-2.0
"""Retain bounded evidence after the owned VM stops, before its disposable disk disappears."""
import contextlib
from datetime import datetime, timezone
import hashlib
import json


@contextlib.contextmanager
def preserve_failure(data, output, name, metadata):
    try:
        yield
    except Exception as original:
        # A session scope can be nested inside the owning disk mission. Keep
        # the first (most specific) failure capture, including an incomplete one.
        if getattr(original, "_rustic_failure_captured", False):
            raise
        original._rustic_failure_captured = True
        evidence = {
            "verified": False, "session": name,
            "captured_at": datetime.now(timezone.utc).isoformat(),
            "kernel_sha256": metadata["kernel_sha256"],
            "build_id": metadata["build_id"],
            "error_type": type(original).__name__, "error": str(original)[:2000],
            "capture_boundary": "owned_vm_stopped_before_disposable_disk_cleanup",
        }
        try:
            # Save raw bytes even if no metadata bank can be parsed. This is an
            # observation after VM shutdown, not a claim about flush durability.
            with data.open("rb") as stream:
                prefix = stream.read(174 * 512)
                stream.seek(-512, 2)
                tail = stream.read(512)
            for suffix, payload in (("files.bin", prefix), ("last-sector.bin", tail)):
                target = output / f"failure-{name}.{suffix}"
                target.write_bytes(payload)
                evidence[suffix] = {"path": target.name, "bytes": len(payload),
                                    "sha256": hashlib.sha256(payload).hexdigest()}
        except Exception as capture:
            evidence["capture_error"] = f"{type(capture).__name__}: {capture}"[:2000]
            original.add_note("Failure disk capture was incomplete: " + evidence["capture_error"])
        try:
            (output / f"failure-{name}.json").write_text(json.dumps(evidence, indent=2) + "\n")
        except Exception as capture:
            original.add_note(f"Failure metadata could not be saved: {type(capture).__name__}")
        raise
