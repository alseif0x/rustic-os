# SPDX-License-Identifier: Apache-2.0
"""Best-effort failure capture cannot promote or replace a failed worker status."""
import json
import subprocess
import tarfile
from .artifacts import unpack_bundle
from .container.export_failure import MAX_BUNDLE, REPORT, allowed_files
from .process import command


def collect(container, mode, directory):
    transfer = directory / "boot-failure-evidence.transfer"
    outcome = None
    try:
        code = command(["docker", "exec", container, "python3", "-I",
                        "/opt/controller/export_failure.py", mode],
                       transfer, timeout=30, limit=MAX_BUNDLE)
        if code:
            raise RuntimeError(f"failure evidence exporter exited with code {code}")
        artifacts = unpack_bundle(transfer.read_bytes(), allowed_files(mode), MAX_BUNDLE)
        if REPORT not in artifacts:
            raise RuntimeError("failure evidence omitted its capture report")
        report = json.loads(artifacts[REPORT])
        if (not isinstance(report, dict) or report.get("schema_version") != 1
                or report.get("mode") != mode or report.get("status") not in ("captured", "partial")
                or not isinstance(report.get("captured"), list)
                or len(report["captured"]) != len(set(report["captured"]))
                or set(report["captured"]) != set(artifacts) - {REPORT}
                or not isinstance(report.get("missing"), list)
                or not isinstance(report.get("errors"), list)):
            raise RuntimeError("invalid failure evidence capture report")
        for name, payload in artifacts.items():
            (directory / name).write_bytes(payload)
        outcome = report
    except (RuntimeError, OSError, ValueError, TypeError, tarfile.TarError, subprocess.SubprocessError) as error:
        outcome = {"status": "failed", "error": f"{type(error).__name__}: {error}"[:2000]}
    finally:
        try:
            transfer.unlink(missing_ok=True)
        except OSError as error:
            if outcome is not None:
                outcome["transfer_cleanup_error"] = str(error)[:2000]
                if outcome["status"] == "captured":
                    outcome["status"] = "partial"
    return outcome
