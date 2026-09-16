# SPDX-License-Identifier: Apache-2.0
"""Prove the declared RAM profiles: build and boot `ok` at 256, 512 and 2048 MiB.

The kernel's frame bitmaps have a fixed address budget (issue #48). A smaller
machine must still account exactly, and a machine with memory above the old
1 GiB limit must allocate, touch and release real frames there. This suite only
records what the guest itself prints; every number below is checked for internal
consistency, so a stale or truncated line cannot pass.
"""
import argparse
import json
import shutil
import subprocess
from pathlib import Path

import environment
from boot_support import scenarios
from boot_support.image import MEMORY_PROFILES, build
from boot_support.runner import run_once

# The declared profiles plus the smallest profile whose usable map passes 2 GiB,
# which is what proves frames above the old boundary. Larger sizes (8 GiB and
# up) need the host to free that much RAM; pass them through --profiles.
PROFILES = MEMORY_PROFILES + (4096,)
TEST_PROFILES = MEMORY_PROFILES + (4096, 6144, 8192, 12288, 16384)
# Boundaries this evidence is stated against: the 1 GiB budget the declared
# profiles replaced, and the current address budget
# (kernel/src/arch/x86_64/memory/physical.rs: LIMIT).
OLD_LIMIT_BYTES = 1 << 30
LIMIT_BYTES = 16 << 30
PAGE_BYTES = 4096
# One bit per page in each of the two static bitmaps, for the fixed budget.
METADATA_BYTES = LIMIT_BYTES // PAGE_BYTES // 8 * 2
DEFAULT_OUTPUT = environment.ROOT / "artifacts/memory-profiles"


def parse(serial):
    """Return the two accounting records, refusing duplicate or missing lines."""
    memory, = scenarios.records(serial, "RUSTIC MEMORY ")
    frames, = scenarios.records(serial, "RUSTIC MEMORY_FRAMES ")
    return ({key: int(value) for key, value in memory.items()},
            {key: int(value) for key, value in frames.items()})


def check(mib, result, serial, memory, frames, managed):
    """Every statement is a consequence of what the guest reported, and the
    boot really used the requested machine size: the recorded QEMU command must
    ask for it, usable memory must sit in that profile's band, and the guest must
    see memory above the old limit exactly when the profile has it."""
    assert result["outcome"] == "success", f"{mib} MiB did not boot: {result['outcome']}"
    assert scenarios.memory_verified(serial), f"{mib} MiB memory fixture did not verify"
    assert scenarios.frames_verified(serial), f"{mib} MiB frame report did not verify"
    assert result["memory_mib"] == mib, f"{mib} MiB: runner reported {result['memory_mib']}"
    assert f"{mib}M" in result["command"], f"{mib} MiB: QEMU was not asked for that size"
    assert memory["limit_bytes"] == LIMIT_BYTES, f"{mib} MiB: unexpected address budget"
    assert memory["metadata_bytes"] == METADATA_BYTES, f"{mib} MiB: metadata cost changed"
    assert frames["usable_bytes"] >= managed, f"{mib} MiB: managed bytes exceed the map"
    assert frames["managed_bytes"] == memory["managed_frames"] * PAGE_BYTES
    assert frames["reserved_bytes"] == frames["usable_bytes"] - frames["managed_bytes"]
    assert frames["allocated_frames"] + frames["free_frames"] == memory["managed_frames"]
    low, high_band = (mib - 64) << 20, mib << 20
    assert low <= frames["usable_bytes"] < high_band, f"{mib} MiB: usable memory outside the profile"
    assert memory["exhausted"] == memory["free_before"] > 0
    # A profile whose usable map passes the old 1 GiB boundary must report the
    # frames it took above it, and the count cannot exceed the frames the highest
    # reported index allows.
    high = frames["usable_bytes"] > OLD_LIMIT_BYTES
    if high:
        span = memory["high_frame"] - memory["high_boundary_frame"] + 1
        assert memory["high_frame"] >= memory["high_boundary_frame"]
        assert 0 < memory["high_frames"] <= span, f"{mib} MiB: high-frame count is impossible"
        # The reference profile carries the recorded regression guard: on this
        # firmware the map keeps a small unmanaged area above the boundary, so the
        # count is the span minus that area. A kernel that reported the span as
        # the count would show a zero (or huge) gap and fail here.
        if mib == 2048:
            assert 0 < span - memory["high_frames"] <= 4096, (
                f"{mib} MiB: high-frame count {memory['high_frames']} against span {span} "
                "does not exclude the map gap above the old boundary; the span alone "
                "is not a count"
            )
    else:
        assert memory["high_frames"] == 0 and memory["high_frame"] == 0
    return high


def source():
    """The revision and worktree the evidence belongs to. Recorded, not inferred:
    the same artifact can otherwise be re-read as current after later edits."""
    return {
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"]).decode().strip(),
        "worktree_status": subprocess.check_output(["git", "status", "--porcelain"]).decode(),
    }


def verify(output=DEFAULT_OUTPUT, profiles=PROFILES):
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    for stale in output.glob("boot_*"):
        shutil.rmtree(stale)
    for stale in output.glob("ok-*"):
        shutil.rmtree(stale)
    (output / "qemu.log").write_text("")
    revision = source()
    boots = {}
    for mib in profiles:
        if mib not in MEMORY_PROFILES and mib not in TEST_PROFILES:
            raise ValueError(f"unsupported memory profile: {mib}")
        # Each profile has its own image directory and its own boot output, so the
        # image is booted from the metadata it was built with and no profile
        # overwrites another's evidence.
        directory = output / f"ok-{mib}"
        directory.mkdir()
        image = build("ok", memory=mib)
        built = json.loads((Path(image).resolve().parent / "image.json").read_text())
        assert built["memory_mib"] == mib, f"image did not carry the {mib} MiB profile"
        # The machine-wide exhaustion walk scales with RAM, so a large profile
        # needs a correspondingly larger budget than the reference 180 s.
        result = run_once(image, max(180, mib), output=directory)
        serial = (directory / "serial.log").read_text(errors="replace")
        memory, frames = parse(serial)
        managed = memory["managed_frames"] * PAGE_BYTES
        high = check(mib, result, serial, memory, frames, managed)
        boots[mib] = {"result": result, "memory": memory, "frames": frames,
                      "image_sha256": built["image_sha256"], "high": high,
                      "image_directory": str(Path(image).resolve().parent),
                      "source_commit": built["source_commit"]}
    evidence = {
        "verified": all(boot["result"]["outcome"] == "success" for boot in boots.values()),
        "revision": revision["revision"],
        "worktree_status": revision["worktree_status"],
        "profiles_mib": list(profiles),
        "address_limit_bytes": LIMIT_BYTES,
        "old_limit_bytes": OLD_LIMIT_BYTES,
        "metadata_bytes": METADATA_BYTES,
        "boots": {str(mib): boot for mib, boot in boots.items()},
    }
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    assert evidence["verified"], "a declared RAM profile did not verify"
    print(json.dumps(evidence, indent=2))
    return evidence


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default=str(DEFAULT_OUTPUT))
    parser.add_argument("--profiles", type=int, nargs="+", default=list(PROFILES))
    args = parser.parse_args()
    verify(args.output, tuple(args.profiles))
