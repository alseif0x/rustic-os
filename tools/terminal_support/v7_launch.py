# SPDX-License-Identifier: Apache-2.0
"""Start storage-sourced utility builds against one unchanged kernel image in QEMU.

One `terminal-v7` image is built once and copied aside. Two utility variants
are built by separate cargo invocations into separate target directories with
different `RUSTIC_UTILITY_TAG` values; neither touches `target/native`, whose
utility is the one embedded in the kernel. Each variant is provisioned on its
own fresh V7 volume and the same frozen image boots once per volume. In each
boot the owner stages the pair, is refused for a role that needs file
authority, starts the child control-only, reads its report (which carries the
build tag), and reaps its exit code.

"Independently built" here means separate build invocations from the same
source producing distinct ELF digests and distinct observable behaviour. It
does not mean separate source trees, toolchains or publishers.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import uuid

import application
import environment
from .connection import Connection
from .machine import machine
from .v7_read import stage_pair, version_text, volume_json


ROOT = environment.ROOT
BOOT_TIMEOUT = 300
EXIT_TIMEOUT = 30
TAGS = (1, 2)
IDENTITY = "rustic.utility"
# Kernel exit kinds (`exit_words` in kernel/src/process/runtime/native/control.rs).
EXIT_CODE, EXIT_FAULT, EXIT_KILLED = 1, 2, 3
FINISH_CODE = 7  # apps/utility/src/actions.rs, role FINISH
INVALID_OPCODE = 6  # x86 #UD raised by role FAULT
ROLE_REFUSAL = "role needs authority the control-only topology does not issue"
STARTED_REFUSAL = "already started"
# Any file name: V7 cannot resolve paths, so a file-access utility is never launched.
LAUNCH_PROBE_FILE = "application/utility.manifest"
PROCESS_ROW = re.compile(r"(?m)^(\d+) (\w+) (\d+) (\d+) (\d+) (\d+) (\S+)\r?$")


def start_outcome(text):
    """Decode one `start-staged` answer; anything else is a local failure."""
    text = text.replace("\r\n", "\n")
    started = re.search(r"(?m)^started staged pid=(\d+) role=(\w+) topology=control-only$", text)
    error = re.search(r"(?m)^error: (.+)$", text)
    if started and not error:
        return {"state": "started", "pid": int(started[1]), "role": started[2]}
    if error and not started:
        refused = re.fullmatch(r"start refused: (.+)", error[1].strip())
        if refused:
            return {"state": "refused", "reason": refused[1]}
        if error[1].strip() == "service denied":
            return {"state": "denied"}
    raise ValueError(f"unrecognized start-staged output: {text!r}")


def permissions_facts(text):
    """Decode one `permissions PID` answer into its seven words."""
    if "\nerror:" in text.replace("\r\n", "\n"):
        raise ValueError(f"permissions refused: {text!r}")
    facts = re.search(
        r"scope=(\d+) rights=(\d+) generation=(\d+) expires=(\d+) report=(\d+) bytes=(\d+) other=(\d+)",
        text,
    )
    if not facts:
        raise ValueError(f"unrecognized permissions output: {text!r}")
    names = ("scope", "rights", "generation", "expires", "report", "bytes", "other")
    return dict(zip(names, map(int, facts.groups())))


def reap_outcome(text):
    """Decode one successful `reap PID` answer as `(exit_kind, code)`."""
    reaped = re.search(r"(?m)^ok exit_kind=(\d+) code=(\d+)\r?$", text)
    if not reaped or "\nerror:" in text.replace("\r\n", "\n"):
        raise ValueError(f"unrecognized reap output: {text!r}")
    return int(reaped[1]), int(reaped[2])


def process_rows(text):
    """Every `ps` row by PID, including exit kind and code."""
    return {
        int(row[1]): {"state": row[2], "kind": int(row[3]), "code": int(row[4]), "program": row[7]}
        for row in PROCESS_ROW.finditer(text)
    }


def counters(text):
    """Process and channel counts from one `mem` answer."""
    found = dict(re.findall(r"\b(processes|channels)=(\d+)", text))
    if set(found) != {"processes", "channels"}:
        raise ValueError(f"unrecognized mem output: {text!r}")
    return {key: int(value) for key, value in found.items()}


def manifest_facts(data):
    """Identity, version and ELF digest of one encoded schema-2 manifest."""
    if len(data) != 128 or data[:8] != b"RUSTAPP\0":
        raise ValueError("not a schema-2 application manifest")
    return {
        "identity": data[32:64].rstrip(b"\0").decode("ascii"),
        "version": [int.from_bytes(data[i:i + 2], "little") for i in (18, 20, 22)],
        "requests": int.from_bytes(data[24:32], "little"),
        "artifact_sha256": data[96:128].hex(),
    }


def build_variant(tag):
    """Build one tagged utility in its own target directory, never `target/native`."""
    target = ROOT / "target" / "v7-launch" / f"utility-tag-{tag}"
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target)
    env["RUSTIC_UTILITY_TAG"] = str(tag)
    output = application.build_one(ROOT, env, False, "utility", "utility.manifest")
    if output == (ROOT / "target" / "native").resolve():
        raise RuntimeError("a utility variant must not replace the embedded utility artifact")
    elf, manifest = output / "utility.elf", output / "utility.manifest"
    facts = manifest_facts(manifest.read_bytes())
    if facts["artifact_sha256"] != environment.digest(elf):
        raise RuntimeError("variant manifest does not bind its ELF")
    return {
        "tag": tag,
        "target_dir": str(target.relative_to(ROOT)),
        "elf": elf,
        "manifest": manifest,
        "elf_sha256": environment.digest(elf),
        "manifest_sha256": environment.digest(manifest),
        "elf_bytes": elf.stat().st_size,
        "identity": facts["identity"],
        "version": facts["version"],
    }


def _freeze(image, directory):
    """Copy the built image and its metadata aside so both boots use these bytes."""
    directory.mkdir()
    frozen = directory / image.name
    shutil.copyfile(image, frozen)
    shutil.copyfile(image.parent / "image.json", directory / "image.json")
    metadata = json.loads((directory / "image.json").read_text())
    if environment.digest(frozen) != metadata["image_sha256"]:
        raise RuntimeError("image differs from its recorded digest")
    # The kernel the firmware will load is the one inside the FAT image.
    kernel = directory / "kernel-from-image.elf"
    subprocess.run(["mcopy", "-n", "-i", str(frozen), "::/kernel.elf", str(kernel)], check=True)
    kernel_sha256 = environment.digest(kernel)
    if kernel_sha256 != metadata["kernel_sha256"]:
        raise RuntimeError("kernel inside the image differs from the recorded kernel digest")
    return frozen, metadata, kernel_sha256


def _processes(uart):
    return process_rows(uart.command("ps"))


def _wait_exit(uart, pid):
    deadline = time.monotonic() + EXIT_TIMEOUT
    while True:
        row = _processes(uart).get(pid)
        if row and row["state"] == "exited":
            return row
        if time.monotonic() > deadline:
            raise AssertionError(f"started staged child {pid} did not exit: {row}")
        time.sleep(0.2)


def _report(uart, pid, expected):
    """The collected report, which may lag the exit by one supervisor turn."""
    deadline = time.monotonic() + EXIT_TIMEOUT
    while True:
        facts = permissions_facts(uart.command(f"permissions {pid}"))
        if facts["report"] == expected or time.monotonic() > deadline:
            return facts
        time.sleep(0.2)


def _start(uart, pid, role, expected=None):
    return start_outcome(uart.command(f"start-staged {pid} {role}", expected))


def _stage(uart, pair):
    outcome = stage_pair(uart, *pair)
    if outcome["state"] != "staged":
        raise AssertionError(f"utility pair was not staged: {outcome}")
    pid = outcome["pid"]
    row = _processes(uart).get(pid)
    if not row or (row["state"], row["program"]) != ("dormant", "staged"):
        raise AssertionError(f"staged child is not a dormant dynamic image: {row}")
    return pid, outcome


def _refusals(uart, pid):
    """Owner requests the supervisor must refuse while the child stays dormant."""
    supervisor = next(p for p, row in _processes(uart).items() if row["program"] == "supervisor")
    refused = []
    for role in ("read", "session"):
        outcome = _start(uart, pid, role, "start refused")
        if outcome != {"state": "refused", "reason": ROLE_REFUSAL}:
            raise AssertionError(f"role {role} was not refused by storage policy: {outcome}")
        refused.append({"case": f"role_{role}", **outcome})
    outcome = _start(uart, supervisor, "exit", "service denied")
    if outcome != {"state": "denied"}:
        raise AssertionError(f"a non-staged PID was not denied: {outcome}")
    refused.append({"case": "not_the_staged_child", "pid": supervisor, **outcome})
    row = _processes(uart).get(pid)
    if not row or row["state"] != "dormant":
        raise AssertionError(f"a refused start changed the staged child: {row}")
    return refused


def _finish_case(uart, pair, tag):
    """Stage, refuse, start FINISH control-only, read the tag, reap exit 7."""
    before = counters(uart.command("mem"))
    pid, staged = _stage(uart, pair)
    dormant = permissions_facts(uart.command(f"permissions {pid}"))
    refusals = _refusals(uart, pid)
    started = _start(uart, pid, "exit")
    if started != {"state": "started", "pid": pid, "role": "exit"}:
        raise AssertionError(f"start-staged did not start the staged child: {started}")
    again = _start(uart, pid, "exit", "start refused")
    if again != {"state": "refused", "reason": STARTED_REFUSAL}:
        raise AssertionError(f"a second start was not refused: {again}")
    row = _wait_exit(uart, pid)
    if (row["kind"], row["code"], row["program"]) != (EXIT_CODE, FINISH_CODE, "staged"):
        raise AssertionError(f"started child did not exit with the FINISH code: {row}")
    report = _report(uart, pid, FINISH_CODE)
    expected = {"scope": 0, "rights": 0, "generation": dormant["generation"], "expires": 0,
                "report": FINISH_CODE, "bytes": tag, "other": 0}
    if report != expected:
        raise AssertionError(f"report does not carry the variant tag {tag}: {report}")
    reaped = reap_outcome(uart.command(f"reap {pid}"))
    if reaped != (EXIT_CODE, FINISH_CODE):
        raise AssertionError(f"reap did not report exit code 7: {reaped}")
    uart.command(f"reap {pid}", "error: service denied")
    after = counters(uart.command("mem"))
    if after != before or "staged" in {row["program"] for row in _processes(uart).values()}:
        raise AssertionError(f"the started staged child left state behind: {before} -> {after}")
    return {
        "pid": pid,
        "stage_host_seconds": staged["host_seconds"],
        "dormant": dormant,
        "refusals_before_start": refusals,
        "started": started,
        "second_start": again,
        "exit": {"kind": row["kind"], "code": row["code"]},
        "report": report,
        "reaped": {"kind": reaped[0], "code": reaped[1]},
        "counters_before": before,
        "counters_after": after,
    }


def _killed_case(uart, pair):
    """A kernel refusal of the start leaves the child reapable and the shell usable.

    Killing the staged child while dormant makes the kernel refuse its control
    channel (CONNECT reports `Full` for any refused channel). It stands in for
    channel-table exhaustion, which `_occupy_utility_slots` shows is unreachable
    in this profile.
    """
    before = counters(uart.command("mem"))
    pid, _ = _stage(uart, pair)
    uart.command(f"kill {pid}", "ok exit_kind=0 code=0")
    outcome = _start(uart, pid, "exit", "start refused")
    if outcome != {"state": "refused", "reason": "kernel Full"}:
        raise AssertionError(f"starting a killed child was not refused by the kernel: {outcome}")
    during = counters(uart.command("mem"))
    if during["channels"] != before["channels"]:
        raise AssertionError(f"a refused start leaked a channel: {before} -> {during}")
    reaped = reap_outcome(uart.command(f"reap {pid}"))
    if reaped != (EXIT_KILLED, 0):
        raise AssertionError(f"killed staged child was not reaped as killed: {reaped}")
    uart.command("echo shell-usable", "shell-usable")
    after = counters(uart.command("mem"))
    if after != before:
        raise AssertionError(f"refused start left state behind: {before} -> {after}")
    return {"pid": pid, "refusal": outcome, "channels_after_refusal": during["channels"],
            "reaped": {"kind": reaped[0], "code": reaped[1]}, "counters_after": after}


def _occupy_utility_slots(uart, before):
    """Fill both utility slots with the only utilities V7 can run: control-only ones.

    A file-access utility cannot be launched in this profile: the shell cannot
    name a file (V7 path resolution is `Unsupported`), and the supervisor refuses
    file-access roles outside the V5 profile anyway. So the channel peak here is
    four resident channels, two utility control channels and one staged-child
    control channel: seven of eight, and real exhaustion is unreachable.
    """
    spins = []
    for _ in range(2):
        spins.append(int(re.search(r"started pid=(\d+)", uart.command("run spin", "started pid="))[1]))
    uart.command("run spin", "error: service busy or full")
    unsupported = uart.command(f"run read {LAUNCH_PROBE_FILE}", "error: Unsupported")
    occupied = counters(uart.command("mem"))
    if occupied != {"processes": before["processes"] + 2, "channels": before["channels"] + 2}:
        raise AssertionError(f"two control-only utilities did not add two channels: {before} -> {occupied}")
    return spins, occupied, "run read" in unsupported


def _fault_case(uart, pair):
    """With both utility slots full, a started child that faults is isolated and reaped.

    The start does not need a utility slot; its channel is the seventh of eight.
    """
    before = counters(uart.command("mem"))
    pid, _ = _stage(uart, pair)
    # The dormant staged child is already a process; it has no channel yet.
    staged = counters(uart.command("mem"))
    if staged != {"processes": before["processes"] + 1, "channels": before["channels"]}:
        raise AssertionError(f"staging opened a channel: {before} -> {staged}")
    spins, occupied, file_access_refused = _occupy_utility_slots(uart, staged)
    started = _start(uart, pid, "fault")
    if started != {"state": "started", "pid": pid, "role": "fault"}:
        raise AssertionError(f"fault role did not start: {started}")
    # The supervisor keeps its end until reap, so the channel counts until then.
    peak = counters(uart.command("mem"))
    if peak["channels"] != occupied["channels"] + 1:
        raise AssertionError(f"the started child did not use exactly one channel: {occupied} -> {peak}")
    row = _wait_exit(uart, pid)
    if (row["kind"], row["code"]) != (EXIT_FAULT, INVALID_OPCODE):
        raise AssertionError(f"fault role did not end with #UD: {row}")
    report = permissions_facts(uart.command(f"permissions {pid}"))
    if report["report"] != 0:
        raise AssertionError(f"a faulting child reported: {report}")
    reaped = reap_outcome(uart.command(f"reap {pid}"))
    if reaped != (EXIT_FAULT, INVALID_OPCODE):
        raise AssertionError(f"faulted staged child reaped wrongly: {reaped}")
    released = counters(uart.command("mem"))
    if released != {"processes": occupied["processes"] - 1, "channels": occupied["channels"]}:
        raise AssertionError(f"reaping the started staged child did not release it: {occupied} -> {released}")
    for spin in spins:
        uart.command(f"kill {spin}", "ok")
        _wait_exit(uart, spin)
        uart.command(f"reap {spin}", "exit_kind=3 code=0")
    after = counters(uart.command("mem"))
    if after != before:
        raise AssertionError(f"faulted staged child left state behind: {before} -> {after}")
    return {"pid": pid, "exit": {"kind": row["kind"], "code": row["code"]},
            "reaped": {"kind": reaped[0], "code": reaped[1]},
            "utility_slots_occupied_by": spins, "file_access_utility_refused_unsupported": file_access_refused,
            "counters_with_utilities": occupied, "counters_after_start": peak, "counters_after": after}


def _boot(image, data, pair, variant, index, temporary, output, transcript, log):
    tag = variant["tag"]
    sock = temporary / f"uart-{tag}.sock"
    image_sha256 = environment.digest(image)
    with machine(image, data, f"unix:{sock},server=on,wait=off", log) as vm:
        uart = Connection(sock, vm, transcript, BOOT_TIMEOUT, output / f"commands-{tag}.jsonl")
        try:
            uart.until()
            uart.send(b"job-status 1\r")
            mount = uart.until()
            if "status=0" not in mount:
                raise AssertionError(f"initial V7 mount job did not succeed: {mount!r}")
            result = {"boot": index, "tag": tag, "image_sha256": image_sha256,
                      "finish": _finish_case(uart, pair, tag)}
            if index == 1:
                result["killed_before_start"] = _killed_case(uart, pair)
            else:
                result["fault"] = _fault_case(uart, pair)
            uart.send(b"exit\r")
            uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
            if vm.wait(timeout=10) != 33:
                raise RuntimeError("unclean terminal-v7 exit")
        finally:
            uart.close()
    return result


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-launch")
    output.mkdir(parents=True, exist_ok=True)
    for name in ("result.json", "terminal-v7-launch.json"):
        (output / name).unlink(missing_ok=True)
    started = time.monotonic()
    serials, logs, boots = [], [], []
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-launch-") as temporary:
            temporary = Path(temporary)
            frozen, metadata, kernel_in_image = _freeze(image, temporary / "image")
            embedded = metadata["native_applications"]["utility"]
            variants = [build_variant(tag) for tag in TAGS]
            # The kernel image embeds target/native/utility.elf; the variant
            # builds must have left that artifact exactly as the image recorded.
            native = ROOT / "target/native"
            if (environment.digest(native / "utility.elf"), environment.digest(native / "utility.manifest")) != (
                    embedded[".elf"], embedded[".manifest"]):
                raise AssertionError("variant builds changed the embedded utility artifact")
            digests = [variant["elf_sha256"] for variant in variants]
            if len(set(digests)) != len(digests) or embedded[".elf"] in digests:
                raise AssertionError("variant ELFs are not distinct from each other and the embedded utility")
            if {(variant["identity"], tuple(variant["version"])) for variant in variants} != {
                    (IDENTITY, tuple(variants[0]["version"]))}:
                raise AssertionError("variants are not builds of the same utility manifest")

            volumes = []
            for index, variant in enumerate(variants, 1):
                data = temporary / f"v7-volume-{variant['tag']}.raw"
                lineage = uuid.uuid4().hex
                seeded = volume_json(volume_tool, "seed7", data, lineage, variant["elf"], variant["manifest"])
                if seeded["lineage"] != lineage or seeded["elf"]["size"] != variant["elf_bytes"]:
                    raise AssertionError("host provisioner returned another pair")
                report_before = volume_json(volume_tool, "report7", data)
                before = environment.digest(data)
                pair = (seeded["workspace"]["text"], seeded["elf"]["resource"],
                        seeded["manifest"]["resource"], version_text(seeded["elf"]["version"]),
                        version_text(seeded["manifest"]["version"]))
                serials.append(output / f"serial-{variant['tag']}.log")
                logs.append(output / f"qemu-{variant['tag']}.log")
                boot = _boot(frozen, data, pair, variant, index, temporary, output, serials[-1], logs[-1])
                after = environment.digest(data)
                if before != after or report_before != volume_json(volume_tool, "report7", data):
                    raise AssertionError("a read-only guest boot changed the V7 volume")
                boots.append(boot)
                volumes.append({"tag": variant["tag"], "lineage": lineage, "bytes": data.stat().st_size,
                                "sha256_before": before, "sha256_after": after})
            if {boot["image_sha256"] for boot in boots} != {metadata["image_sha256"]}:
                raise AssertionError("the boots did not use the recorded image bytes")
            if environment.digest(frozen) != metadata["image_sha256"]:
                raise AssertionError("the frozen image changed during the boots")

        evidence = {
            "verified": True,
            "mode": "terminal-v7-launch",
            "topology": "control-only",
            "independent_build_meaning": (
                "separate cargo invocations and target directories from the same source, "
                "differing only in RUSTIC_UTILITY_TAG; distinct ELF digests and FINISH report tags"
            ),
            "image": {
                "build_id": metadata["build_id"],
                "source_commit": metadata["source_commit"],
                "source_status_clean": metadata["source_status"] == "",
                "image_sha256": metadata["image_sha256"],
                "kernel_sha256": metadata["kernel_sha256"],
                "kernel_in_image_sha256": kernel_in_image,
                "boots_with_identical_image": len(boots),
            },
            "embedded_utility": {"elf_sha256": embedded[".elf"], "manifest_sha256": embedded[".manifest"]},
            "variants": [{key: value for key, value in variant.items() if key not in ("elf", "manifest")}
                         for variant in variants],
            "volumes": volumes,
            "boots": boots,
            "unexercised": ["channel-table exhaustion: unreachable in the V7 profile; V7 path resolution is "
                            "Unsupported and the supervisor admits file-access utilities only in V5, so the "
                            "peak is 4 resident + 2 control-only utility + 1 staged control = 7 of 8 channels "
                            "(recorded in boots[1].fault)",
                            "features refusal 13 (no shipped utility manifest omits ipc; host-tested in "
                            "rustic_supervisor::storage_launch)"],
            "identity_refusal_exercised_by": "tools/v7_read_test.py (staged file-server)",
        }
        (output / "terminal-v7-launch.json").write_text(json.dumps(evidence, indent=1) + "\n")
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "kernel_sha256": metadata["kernel_sha256"],
            "terminal_v7_launch": evidence,
        }
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print(f"V7 launch acceptance: one image sha256={metadata['image_sha256']} "
              f"kernel sha256={metadata['kernel_sha256']} booted {len(boots)} times.", flush=True)
        for variant, boot in zip(variants, boots):
            finish = boot["finish"]
            print(f"  tag={variant['tag']} elf sha256={variant['elf_sha256']} pid={finish['pid']} "
                  f"report={finish['report']['report']} tag_word={finish['report']['bytes']} "
                  f"reap code={finish['reaped']['code']}", flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(b"\n".join(path.read_bytes() for path in serials if path.exists()))
        (output / "qemu.log").write_bytes(b"\n".join(path.read_bytes() for path in logs if path.exists()))
