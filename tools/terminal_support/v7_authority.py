# SPDX-License-Identifier: Apache-2.0
"""Owner revocation during an in-flight V7 admission publication, in two disposable UEFI boots.

Boot 1 admits a 4 KiB pattern into the seeded `scratch.bin`, then runs
`execute-admission-v7 ID revoke 0 TICKS`: the shell arms the kernel's
completion-hold diagnostic for the file service, sends EXECUTE without waiting,
waits until the service's first publication write is held, and has the owner
revoke the shell's binding (`REVOKE_SHELL_V7`). The held write precedes the
header, so the service stops the execution, records the admission cancelled
with cause `authority_lost` and only then acknowledges the revocation. The old
endpoint reports `Uncertain` (closed with the reply outstanding); on the new
binding the status is `cancelled`, the observation shows `authority_lost`, and
`oracle7` finds a cancelled record with that cause, the live file unchanged and
exactly one new generation (the prevention). Execute, cancel and status
replays print the same status without writing.

A second admission is revoked the same way during its ACCEPT, whose first
publication command (the payload flush) is held: nothing is published, no
record exists for its key and the file is unchanged. Admitting the same key
again on the new binding succeeds, so the revoked stage released its
reservation, and executing it commits the pattern through the pollable path.

Boot 2 finds the cancelled admission and its cause unchanged.

A revocation after the header cannot be held in the guest: the hold diagnostic
skips at most 16 earlier mutations, and a V7 publication writes 100 metadata
sectors before its header. That case is covered by host tests only.
"""
import hashlib
import json
from pathlib import Path
import re
import tempfile
import time
import uuid

import environment
from . import oracle7
from .v7_admission import _admit_command, _expected, _file, check_record, decode_observation, decode_status
from .v7_read import volume_json
from .v7_retention import view
from .v7_write import boot_terminal, check_absent, hex16, pattern


ROOT = environment.ROOT
SEEDED_RECORDS = 2
SIZE = 4096
HOLD_TICKS = 200
EXECUTE_KEY, ACCEPT_KEY = 0x500, 0x501
EXECUTE_SEED, ACCEPT_SEED = 4, 5

REVOKE = re.compile(r"^revoke-v7 held=(0|1) job=([1-9][0-9]{0,15}) old=([A-Za-z]+) "
                    r"ticks=(0|[1-9][0-9]{0,15})$")


def decode_revoke(text):
    """Decode the diagnostic's revocation line and the outcome printed after it."""
    lines = [line.strip() for line in text.replace("\r\n", "\n").split("\n")]
    heads = [REVOKE.match(line) for line in lines if line.startswith("revoke-v7")]
    if len(heads) != 1 or not heads[0]:
        errors = [line for line in lines if line.startswith("error")]
        raise ValueError(f"no revocation line: {errors or text!r}")
    match = heads[0]
    return {"held": match[1] == "1", "job": int(match[2]), "old": match[3], "ticks": int(match[4]),
            "outcome": decode_status(text)}


def check_revoked(revoked, what):
    """The publication was held when the owner revoked, and the old endpoint lost its reply."""
    if not revoked["held"]:
        raise AssertionError(f"{what}: the publication command was not held before the revocation")
    if revoked["old"] != "Uncertain":
        raise AssertionError(f"{what}: the revoked endpoint reported {revoked['old']}, expected Uncertain")


def _live(data):
    return oracle7.snapshot(data.read_bytes())


def _status(uart, text, timing=False):
    return decode_status(uart.command(text, "\n"), timing)


def _same(uart, commands, status, data, what):
    digest = environment.digest(data)
    for text in commands:
        again = _status(uart, text)
        if again.get("lines") != status["lines"]:
            raise AssertionError(f"{what}: {text!r} printed {again}, expected {status['lines']}")
    if environment.digest(data) != digest:
        raise AssertionError(f"{what} changed the image")


def _execute_cut(uart, data, refs, ids, state, evidence):
    content = pattern(EXECUTE_SEED, SIZE)
    admitted = _status(uart, _admit_command(refs, state["version"], state["epoch"], EXECUTE_KEY, EXECUTE_SEED,
                                            SIZE), timing=True)
    if admitted.get("state") != "admitted":
        raise AssertionError(f"the admission to execute was not admitted: {admitted}")
    before = _live(data)
    started = time.monotonic()
    revoked = decode_revoke(uart.command(f"execute-admission-v7 {admitted['id']} revoke 0 {HOLD_TICKS}", "\n"))
    revoked["host_seconds"] = round(time.monotonic() - started, 3)
    check_revoked(revoked, "execution")
    cancelled = revoked["outcome"]
    if cancelled.get("state") != "cancelled" or cancelled["number"] != admitted["number"] \
            or cancelled["instance"] != admitted["instance"]:
        raise AssertionError(f"the revoked execution was not cancelled: {cancelled}")
    observed = decode_observation(uart.command(f"observe-admission-v2 {admitted['id']}", "\n"))
    if (observed["state"], observed["prevention"], observed["terminal"]) != \
            ("cancelled", "authority_lost", cancelled["terminal"]):
        raise AssertionError(f"observation of the revoked execution: {observed}")
    live = _live(data)
    record = check_record(live, cancelled, {**_expected(ids, state["epoch"], EXECUTE_KEY, state["version"], content),
                                            "cause": "authority_lost"})
    if _file(live, ids[1]) != _file(before, ids[1]):
        raise AssertionError("the revoked execution changed the live file")
    # The execution published nothing; the prevention is the one new generation.
    if live["sequence"] != before["sequence"] + 1 or cancelled["terminal"] != live["sequence"]:
        raise AssertionError("the revoked execution published more than its prevention")
    _same(uart, [f"execute-admission {admitted['id']}", f"cancel-admission {admitted['id']}",
                 f"admission {admitted['id']}"], cancelled, data, "cancelled replays")
    evidence.update({"admitted": admitted, "revoked": {k: revoked[k] for k in ("held", "job", "old", "ticks",
                                                                                 "host_seconds")},
                     "cancelled": cancelled, "observation": observed, "record": record,
                     "file_unchanged": True, "generations": 1, "replays": "identical", "oracle": view(live)})
    return cancelled


def _accept_cut(uart, data, refs, ids, state, evidence):
    content = pattern(ACCEPT_SEED, SIZE)
    admit = _admit_command(refs, state["version"], state["epoch"], ACCEPT_KEY, ACCEPT_SEED, SIZE)
    before = _live(data)
    revoked = decode_revoke(uart.command(f"{admit} revoke 0 {HOLD_TICKS}", "\n"))
    check_revoked(revoked, "acceptance")
    if revoked["outcome"] != {"error": "OutcomeUnknown"}:
        raise AssertionError(f"the revoked acceptance left an admission: {revoked['outcome']}")
    live = _live(data)
    check_absent(live, ACCEPT_KEY)
    if live["sequence"] != before["sequence"] or _file(live, ids[1]) != _file(before, ids[1]):
        raise AssertionError("the revoked acceptance published a generation or changed the file")
    # The revoked stage released its reservation: the same key admits afresh.
    admitted = _status(uart, admit, timing=True)
    if admitted.get("state") != "admitted" or admitted["number"] != live["sequence"] + 1:
        raise AssertionError(f"the admission after the revoked acceptance: {admitted}")
    started = time.monotonic()
    executed = _status(uart, f"execute-admission {admitted['id']}")
    seconds = round(time.monotonic() - started, 3)
    if executed.get("state") != "committed":
        raise AssertionError(f"execution through the pollable path did not commit: {executed}")
    after = _live(data)
    check_record(after, executed, _expected(ids, state["epoch"], ACCEPT_KEY, state["version"], content))
    scratch = _file(after, ids[1])
    if (scratch["version"], scratch["sha256"]) != (executed["terminal"], hashlib.sha256(content).hexdigest()):
        raise AssertionError("the live file is not the executed admission")
    state["version"] = executed["terminal"]
    evidence.update({"revoked": {k: revoked[k] for k in ("held", "job", "old", "ticks")},
                     "outcome": "OutcomeUnknown", "no_record": True, "no_generation": True,
                     "readmitted": admitted, "executed": executed, "execute_host_seconds": seconds,
                     "oracle": view(after)})


def _first_boot(uart, data, refs, ids, state, evidence):
    execute, accept = {}, {}
    cancelled = _execute_cut(uart, data, refs, ids, state, execute)
    _accept_cut(uart, data, refs, ids, state, accept)
    evidence.update({"execute_cut": execute, "accept_cut": accept})
    return cancelled


def _second_boot(uart, data, cancelled, evidence):
    digest = environment.digest(data)
    again = _status(uart, f"admission {cancelled['id']}")
    if again.get("lines") != cancelled["lines"]:
        raise AssertionError(f"the cancelled status changed across reboot: {again}")
    observed = decode_observation(uart.command(f"observe-admission-v2 {cancelled['id']}", "\n"))
    if (observed["state"], observed["prevention"]) != ("cancelled", "authority_lost"):
        raise AssertionError(f"the cause changed across reboot: {observed}")
    if environment.digest(data) != digest:
        raise AssertionError("status queries after reboot changed the image")
    evidence.update({"status_after_reboot": "identical", "observation": observed})


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-authority")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    for path, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(path) != metadata["native_applications"]["file-server"][suffix]:
            raise RuntimeError(f"{path.name} differs from the artifact recorded by the boot build")
    (output / "result.json").unlink(missing_ok=True)
    started = time.monotonic()
    phases = [1, 2]
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-authority-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-volume.raw"
            lineage = uuid.uuid4().hex
            seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
            scratch = seeded["scratch"]
            refs = {"workspace": seeded["workspace"]["text"], "resource": scratch["resource"]}
            ids = (seeded["workspace"]["id"], scratch["id"])
            initial = oracle7.snapshot(data.read_bytes())
            if len(initial["records"]) != SEEDED_RECORDS:
                raise AssertionError(f"seed7 provisioned {len(initial['records'])} records")
            state = {"epoch": initial["epoch"], "version": scratch["version"]}
            first, second = {}, {}
            cancelled = boot_terminal(image, data, output, 1, temporary,
                                      lambda uart: _first_boot(uart, data, refs, ids, state, first))
            offline = oracle7.snapshot(data.read_bytes())
            record = check_record(offline, cancelled, {
                **_expected(ids, state["epoch"], EXECUTE_KEY, scratch["version"],
                            pattern(EXECUTE_SEED, SIZE)), "cause": "authority_lost"})
            first["after_shutdown"] = {"record": record, "oracle": view(offline)}
            report = volume_json(volume_tool, "report7", data)
            if report["sequence"] != offline["sequence"]:
                raise AssertionError("report7 and oracle7 disagree after boot 1")
            boot_terminal(image, data, output, 2, temporary,
                          lambda uart: _second_boot(uart, data, cancelled, second))
            final = oracle7.snapshot(data.read_bytes())
            if final["sequence"] != offline["sequence"]:
                raise AssertionError("boot 2 published a generation")
            second["after_shutdown"] = view(final)
        evidence = {"verified": True, "mode": "terminal-v7", "boots": 2, "lineage": lineage,
                    "hold_ticks": HOLD_TICKS, "scratch": scratch, "initial": view(initial),
                    "boot_1": first, "boot_2": second}
        result = {"outcome": "success", "returncode": 33, "timed_out": False,
                  "elapsed_seconds": round(time.monotonic() - started, 3), "build_id": metadata["build_id"],
                  "image_sha256": metadata["image_sha256"], "terminal_v7_authority": evidence}
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        execute = first["execute_cut"]
        print("V7 authority acceptance: an owner revocation while EXECUTE's first publication write was held "
              "stopped it before the header, recorded cancelled/authority_lost (oracle7) with the file "
              "unchanged and one new generation, and survived reboot; a revocation during ACCEPT left no "
              "record or generation, and the same key then admitted and executed.", flush=True)
        print(f"V7 authority timing: revoke_ticks={execute['revoked']['ticks']} "
              f"accept_revoke_ticks={first['accept_cut']['revoked']['ticks']} "
              f"execute_host_seconds={first['accept_cut']['execute_host_seconds']}", flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(
            b"\n".join((output / f"serial-{p}.log").read_bytes() for p in phases if (output / f"serial-{p}.log").exists()))
        (output / "qemu.log").write_bytes(
            b"\n".join((output / f"qemu-{p}.log").read_bytes() for p in phases if (output / f"qemu-{p}.log").exists()))
