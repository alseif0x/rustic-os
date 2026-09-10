# SPDX-License-Identifier: Apache-2.0
"""Versioned metrics and robust, deliberately conservative comparison."""
import hashlib
import json
import math
import re
import statistics

VERSION = 1
# Absolute noise floors are engineering tolerances, not confidence intervals.
METRICS = {
    "boot_ready_seconds": .25,
    "read_seconds": .05,
    "pressure_write_seconds": .05,
    "pressure_control_seconds": .05,
    "revoke_seconds": .05,
    "full_control_seconds": .05,
    "stopped_control_seconds": .05,
    "interrupt_seconds": .05,
    "admitted_control_seconds": .05,
    "drain_seconds": .25,
    "kernel_load_bytes": 4096,
    "kernel_page_table_frames": 1,
    "allocator_metadata_bytes": 0,
    "manager_metadata_bytes": 0,
    "process_fixture_peak_frames": 1,
    "resident_runtime_frames": 1,
    "sampled_pressure_extra_frames": 1,
}
TIMINGS = {name for name in METRICS if name.endswith("_seconds")}
REQUIRED_ARTIFACTS = {
    "probe/serial.log", "probe/qemu.log", "probe/result.json",
    "serial.log", "qemu.log", "files.bin",
}


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                     allow_nan=False).encode()).hexdigest()


def distribution(values):
    if not values or any(isinstance(v, bool) or not isinstance(v, (int, float))
                         or v < 0 or v > 2 ** 53 or not math.isfinite(v) for v in values):
        raise ValueError("metrics require finite nonnegative numeric samples")
    median = statistics.median(values)
    return {"count": len(values), "min": min(values), "median": median,
            "max": max(values), "mad": statistics.median(abs(v - median) for v in values)}


def _require_hex(value, length, label):
    if not isinstance(value, str) or re.fullmatch(rf"[0-9a-f]{{{length}}}", value) is None:
        raise ValueError(f"missing or invalid {label}")


def _source_evidence(source):
    if not isinstance(source, dict):
        raise ValueError("missing source evidence")
    _require_hex(source.get("revision"), 40, "source revision")
    _require_hex(source.get("build_id"), 16, "source build ID")
    _require_hex(source.get("kernel_sha256"), 64, "kernel SHA-256")
    if not isinstance(source.get("worktree_status"), str):
        raise ValueError("missing source worktree status")
    images = source.get("image_sha256")
    if not isinstance(images, dict) or set(images) != {"ok", "terminal"}:
        raise ValueError("missing or unknown source image evidence")
    for name, digest in images.items():
        _require_hex(digest, 64, f"{name} image SHA-256")


def _sample_evidence(sample):
    # Validate portable evidence records, without claiming to authenticate the
    # report or reading artifact paths supplied by a comparison caller.
    _require_hex(sample.get("disk_sha256"), 64, "sample disk SHA-256")
    artifacts = sample.get("artifacts")
    if not isinstance(artifacts, dict) or not REQUIRED_ARTIFACTS <= set(artifacts):
        raise ValueError("missing required sample artifacts")
    for name, digest in artifacts.items():
        _require_hex(digest, 64, f"{name} artifact SHA-256")
    if sample["disk_sha256"] != artifacts["files.bin"]:
        raise ValueError("sample disk and files.bin artifact hashes differ")


def admitted(report):
    if report["schema"] != VERSION or report["configuration_id"] != fingerprint(report["configuration"]):
        raise ValueError("invalid report configuration")
    _source_evidence(report["source"])
    if type(report["injected_delay_ticks"]) is not int or report["injected_delay_ticks"] not in (0, 100):
        raise ValueError("unknown injection profile")
    count = report["requested_samples"]
    if type(count) is not int or not 5 <= count <= 30:
        raise ValueError("five to thirty measured boots are required")
    samples = report["samples"]
    if (len(samples) != count + 1 or any(type(s["warmup"]) is not bool for s in samples)
            or [s["warmup"] for s in samples] != [True] + [False] * count):
        raise ValueError("missing samples or invalid warmup order")
    if any(s["status"] != "success" for s in samples):
        raise ValueError("failed or timed-out attempts cannot enter a baseline")
    for sample in samples:
        _sample_evidence(sample)
        if any(sample.get(key) != report["configuration_id"] for key in ("configuration_before", "configuration_after")):
            raise ValueError("sample configuration drift or missing observation")
        if sample.get("vm_boots_started") != 2:
            raise ValueError("sample did not start both native fixtures")
        if set(sample["metrics"]) != set(METRICS):
            raise ValueError("incomplete or unknown metric set")
        for value in sample["metrics"].values():
            distribution([value])
    return {name: distribution([s["metrics"][name] for s in samples[1:]]) for name in METRICS}


def compare(baseline, candidate):
    try:
        if baseline["status"] != "success" or candidate["status"] != "success":
            raise ValueError("only complete successful collections can be compared")
        left, right = admitted(baseline), admitted(candidate)
        if baseline["injected_delay_ticks"] != 0:
            raise ValueError("an injected run cannot calibrate a baseline")
        if baseline["configuration"] != candidate["configuration"]:
            raise ValueError("configuration/workload/host changed; rebaseline separately")
        if baseline["requested_samples"] != candidate["requested_samples"]:
            raise ValueError("sample counts differ")
    except (KeyError, TypeError, ValueError) as error:
        return {"status": "incomparable", "reason": str(error)}
    metrics = {}
    for name, floor in METRICS.items():
        base, actual = left[name], right[name]
        # Timings use median + max(50%, 6 MAD, absolute floor). Exact counters
        # use maximum + explicit integer allowance, never percentage growth.
        margin = max(.5 * base["median"], 6 * base["mad"], floor) if name in TIMINGS else floor
        limit = (base["median"] if name in TIMINGS else base["max"]) + margin
        observed = actual["median"] if name in TIMINGS else actual["max"]
        metrics[name] = {"baseline": base, "candidate": actual, "upper_limit": limit,
                         "observed": observed, "regression": observed > limit}
    return {"schema": VERSION, "status": "regression" if any(m["regression"] for m in metrics.values()) else "pass",
            "baseline_revision": baseline["source"]["revision"],
            "candidate_revision": candidate["source"]["revision"],
            "configuration_id": baseline["configuration_id"], "metrics": metrics}
