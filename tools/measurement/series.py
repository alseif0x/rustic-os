# SPDX-License-Identifier: Apache-2.0
"""Build once, freeze images, preserve every attempt and compare held-out batches."""
import fcntl
import json
from pathlib import Path
import re
import shutil
import subprocess
from boot_support import image
import environment
from .model import VERSION, admitted, compare, fingerprint
from .provenance import configuration
from .sample import collect


def checked_sample(images, output, warmup, injected, config):
    """Keep environment/source drift out of a homogeneous batch."""
    expected = fingerprint(config)
    def observe():
        try:
            return fingerprint(configuration(environment.ROOT, output.parent, config["host"]["label"])), None
        except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
            return None, f"{type(error).__name__}: {error}"[:2000]

    before, before_error = observe()
    if before != expected:
        output.mkdir()
        result = {"warmup": warmup, "status": "invalid", "metrics": {},
                  "error": before_error or "configuration changed before sample; no VM started", "vm_boots_started": 0}
    else:
        result = collect(images, output, warmup, injected)
    after, after_error = observe()
    result["configuration_before"] = before
    result["configuration_after"] = after
    if after != expected:
        result.update(status="invalid", error=after_error or "configuration changed during sample")
    write(output / "sample.json", result)
    return result


def write(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def prepare(output):
    original = image.build("terminal-init")
    source = json.loads((original.parent / "image.json").read_text())
    kernel = output / "input-kernel.elf"
    shutil.copyfile(original.parent / "kernel.elf", kernel)
    previous = image.OUTPUT
    try:
        image.OUTPUT = output / "images"
        provenance = {key: source[key] for key in ("source_commit", "source_status", "rustc") if key in source}
        images = {"terminal": image.package(kernel, "terminal-init", source["build_id"], provenance),
                  "ok": image.package(kernel, "ok", source["build_id"], provenance)}
    finally:
        image.OUTPUT = previous
    return images, {"revision": source["source_commit"], "worktree_status": source["source_status"],
                    "build_id": source["build_id"], "kernel_sha256": environment.digest(kernel),
                    "image_sha256": {name: environment.digest(p) for name, p in images.items()}}


def batch(images, source, config, directory, samples, injected):
    directory.mkdir()
    report = {"schema": VERSION, "configuration": config, "configuration_id": fingerprint(config),
              "source": source, "requested_samples": samples, "injected_delay_ticks": injected,
              "samples": [], "status": "incomplete"}
    for index in range(samples + 1):
        result = checked_sample(images, directory / f"sample-{index:02}", index == 0, injected, config)
        report["samples"].append(result)
        write(directory / "report.json", report)
        print(json.dumps({"batch": directory.name, "sample": index, "warmup": index == 0,
                          "status": result["status"]}), flush=True)
    try:
        report["summary"] = admitted(report)
        report["status"] = "success"
    except (KeyError, TypeError, ValueError) as error:
        report["error"] = str(error)
    write(directory / "report.json", report)
    return report


def execute(output, label, samples, verification=False, injected=0):
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,63}", label):
        raise ValueError("host label must contain 1-64 ASCII letters/digits/dot/underscore/hyphen")
    if not 5 <= samples <= 30 or injected not in (0, 100):
        raise ValueError("samples must be 5-30; injected delay is 0 or 100 PIT ticks")
    root = environment.ROOT
    (root / ".cache").mkdir(exist_ok=True)
    with (root / ".cache/measurement.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        output = Path(output).resolve()
        output.mkdir(parents=True, exist_ok=False)
        config = configuration(root, output, label)
        images, source = prepare(output)
        if not verification:
            return batch(images, source, config, output / "run", samples, injected)
        baseline = batch(images, source, config, output / "baseline", samples, 0)
        control = batch(images, source, config, output / "control", samples, 0)
        regression = batch(images, source, config, output / "injected", samples, 100)
        control_result, regression_result = compare(baseline, control), compare(baseline, regression)
        write(output / "control-comparison.json", control_result)
        write(output / "regression-comparison.json", regression_result)
        detected = (regression_result["status"] == "regression"
                    and regression_result["metrics"]["read_seconds"]["regression"])
        result = {"schema": VERSION, "status": "success" if control_result["status"] == "pass" and detected else "failure",
                  "source": source, "configuration_id": fingerprint(config),
                  "control_status": control_result["status"], "regression_status": regression_result["status"],
                  "injected_read_regression_detected": detected, "measured_samples_per_batch": samples,
                  "warmups_per_batch": 1, "native_vm_boots": sum(s["vm_boots_started"] for b in (baseline, control, regression) for s in b["samples"])}
        write(output / "verification.json", result)
        return result
