# SPDX-License-Identifier: Apache-2.0
"""Native acceptance of the original receipt/retry API on disposable V7 media.

The fixture is a fresh ``seed7 --scratch`` image. It runs the old shell
``retry-key``, ``replace`` and ``receipt`` commands over two terminal boots,
then checks the shared V7 records and both current and retained bytes with the
independent ``oracle7`` reader. The same run checks mounted capability
discovery and confirms that lifecycle negotiation and read-only discovery leave
the image unchanged.
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
from . import v7_profile1 as profile1
from .discovery_cases import METHODS, capabilities as read_capabilities
from .operation_cases import check as check_operation
from .negotiation_cases import profiles
from .recovery_cases import stat as stat_file
from .v7_read import volume_json
from .v7_write import boot_terminal


ROOT = environment.ROOT
SHELL_SUBJECT = 2
TOP_LEVEL_WORKSPACES = 4
SEEDED_RECORDS = 2
RETAINED = 8
SCRATCH_NAME = "scratch.bin"
OTHER_NAME = "legacy-other.bin"

LEGACY_KEY = 0x701
SHARED_KEY = 0x702
AMBIGUOUS_KEY = 0x703
MISSING_KEY = 0x704

LEGACY_CONTENT = b"old recovery commit"
MODERN_CONTENT = b"modern shared commit"
AMBIGUOUS_LEGACY_CONTENT = b"old side of a collision"
AMBIGUOUS_MODERN_CONTENT = b"new side of a collision"
CHANGED_CONTENT = b"old recovery commix"

CAPABILITY_METHODS = {
    "capabilities.list": "degraded",
    "capabilities.describe": "unavailable",
    "files.read": "available",
    "files.replace": "available",
    "operations.get": "available",
    "operations.cancel": "unavailable",
    "events.read": "unavailable",
    "system.status": "unavailable",
}
CAPABILITY_BOUNDS = {
    "max_inline_bytes": 1024,
    "max_page_items": len(METHODS),
    "receipt_capacity": RETAINED,
}
RETRY_TOKEN = re.compile(r"^[0-9a-f]{64}$")
OLD_RECEIPT = re.compile(
    r"^committed id=([1-9][0-9]{0,9}) previous=([1-9][0-9]{0,19}) "
    r"version=([1-9][0-9]{0,19}) bytes=(0|[1-9][0-9]{0,3})$"
)
MAINTENANCE = re.compile(
    r"^maintain-v7 previous=e_([0-9a-f]{16}) epoch=e_([0-9a-f]{16}) "
    r"records=(0|[1-9][0-9]*) sectors=(0|[1-9][0-9]*) "
    r"job=([1-9][0-9]*) ticks=(0|[1-9][0-9]*)$"
)


def _lines(text):
    return [line.strip() for line in text.replace("\r\n", "\n").split("\n")]


def _error(text, name, what):
    lines = _lines(text)
    errors = [line for line in lines if line.startswith("error:")]
    receipts = [line for line in lines
                if line.startswith(("committed ", "operation-v1 ", "receipt workspace="))]
    if errors != [f"error: {name}"] or receipts:
        raise AssertionError(f"{what}: expected one {name} refusal, got {text!r}")
    return name


def _retry_token(uart, path, key, lineage, epoch):
    text = uart.command(f"retry-key {path} {key}", "retry-key=")
    values = [line[len("retry-key="):] for line in _lines(text) if line.startswith("retry-key=")]
    if len(values) != 1 or not RETRY_TOKEN.fullmatch(values[0]):
        raise AssertionError(f"retry-key returned a missing or malformed token: {text!r}")
    token = values[0]
    raw = bytes.fromhex(token)
    if raw[:16].hex() != lineage or int.from_bytes(raw[16:24], "little") != epoch \
            or int.from_bytes(raw[24:32], "little") != key:
        raise AssertionError("retry-key token does not encode the mounted lineage, epoch and chosen key")
    return token


def _foreign_token(token):
    raw = bytearray.fromhex(token)
    raw[0] ^= 0x80
    return raw.hex()


def _old_replace_command(path, previous, token, content):
    text = content.decode("ascii")
    if any(char in text for char in ('"', "\r", "\n")):
        raise ValueError("legacy recovery acceptance content must fit one plain shell argument")
    return f'replace {path} {previous} {token} "{text}"'


def _old_receipt(text, what):
    lines = [line for line in _lines(text) if line.startswith("committed ")]
    errors = [line for line in _lines(text) if line.startswith("error:")]
    if len(lines) != 1 or errors:
        raise AssertionError(f"{what}: old SDK did not print exactly one committed receipt: {text!r}")
    match = OLD_RECEIPT.fullmatch(lines[0])
    if not match:
        raise AssertionError(f"{what}: old SDK receipt changed its canonical text: {lines[0]!r}")
    return {
        "line": lines[0],
        "id": int(match[1]),
        "previous": int(match[2]),
        "version": int(match[3]),
        "bytes": int(match[4]),
    }


def _receipt_line(identity, previous, version, length):
    return f"committed id={identity} previous={previous} version={version} bytes={length}"


def _old_replace(uart, path, previous, token, content, what):
    command = _old_replace_command(path, previous, token, content)
    receipt = _old_receipt(uart.command(command, "committed id="), what)
    if receipt["line"] != _receipt_line(receipt["id"], previous, receipt["version"], len(content)):
        raise AssertionError(f"{what}: old receipt does not echo the requested previous version and size")
    return {"command": command, "receipt": receipt}


def _old_lookup(uart, identity, token, what):
    return _old_receipt(uart.command(f"receipt {identity} {token}", "committed id="), what)


def _check_old_receipt(receipt, identity, previous, version, content, what):
    expected = {
        "id": identity,
        "previous": previous,
        "version": version,
        "bytes": len(content),
        "line": _receipt_line(identity, previous, version, len(content)),
    }
    if receipt != expected:
        raise AssertionError(f"{what}: old SDK receipt {receipt!r} differs from {expected!r}")


def _snapshot(data):
    return oracle7.snapshot(data.read_bytes())


def _records(snapshot, expected, what):
    count = len(snapshot["records"])
    if count != expected:
        raise AssertionError(f"{what}: oracle7 finds {count} retained records, expected {expected}")
    if count > RETAINED:
        raise AssertionError(f"{what}: retained records exceed the V7 limit of {RETAINED}")


def _file(snapshot, identity):
    found = [item for item in snapshot["files"] if item["id"] == identity]
    if len(found) != 1:
        raise AssertionError(f"oracle7 finds {len(found)} live files with identity {identity}")
    return found[0]


def _check_live(snapshot, identity, version, content, what):
    node = _file(snapshot, identity)
    expected = {
        "version": version,
        "size": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
    }
    if any(node[field] != value for field, value in expected.items()):
        raise AssertionError(f"{what}: oracle7 live metadata differs from the receipt")
    if snapshot["contents"].get(node["path"]) != content:
        raise AssertionError(f"{what}: oracle7 decoded different live bytes")
    return node


def _check_legacy_record(snapshot, object_id, epoch, key, previous, committed, content, what):
    found = [record for record in snapshot["records"]
             if record["subject"] == SHELL_SUBJECT
             and record["workspace"] == TOP_LEVEL_WORKSPACES
             and record["object"] == object_id
             and record["epoch"] == epoch
             and record["key"] == key]
    if len(found) != 1:
        raise AssertionError(f"{what}: oracle7 finds {len(found)} matching canonical-root legacy records")
    record = found[0]
    expected = {
        "state": "direct_committed",
        "previous": previous,
        "committed": committed,
        "terminal": committed,
        "admission": 0,
        "length": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
    }
    if any(record[field] != value for field, value in expected.items()):
        raise AssertionError(f"{what}: oracle7 record differs from the old SDK receipt")
    if not 0 < record["instance"] <= committed or record["epoch"] > committed:
        raise AssertionError(f"{what}: oracle7 legacy record has an invalid V7 instance")
    return {field: record[field] for field in ("slot", "workspace", "object", "state", "previous",
                                               "committed", "length", "sha256")}


def _legacy_matches_profile1(receipt, operation, identity, content, what):
    modern = operation["receipt"]
    expected = {
        "id": identity,
        "previous": int(modern["previous_version"][2:], 16),
        "version": int(modern["version"][2:], 16),
        "bytes": len(content),
        "line": _receipt_line(
            identity,
            int(modern["previous_version"][2:], 16),
            int(modern["version"][2:], 16),
            len(content),
        ),
    }
    if receipt != expected:
        raise AssertionError(f"{what}: legacy and profile-1 receipts do not name the same V7 effect")
    return expected["line"]


def _discovery(uart, data, what):
    before = environment.digest(data)
    report = read_capabilities(uart)
    observed = {method: report[method] for method in METHODS}
    if observed != CAPABILITY_METHODS:
        raise AssertionError(f"{what}: capability catalogue differs from the V7 implementation: {observed!r}")
    if report["bounds"] != CAPABILITY_BOUNDS:
        raise AssertionError(f"{what}: capability limits differ from the mounted V7 profile: {report['bounds']!r}")

    lifecycle = profiles(uart, True)
    for descriptor in lifecycle:
        expected = {"version": 2, "profile": 1, "retained": RETAINED,
                    "tickets": 2, "active": 1}
        if any(descriptor[key] != value for key, value in expected.items()) \
                or descriptor["responder"] <= 0 or descriptor["context"] <= 0 \
                or not re.fullmatch(r"[0-9a-f]{64}", descriptor["sha256"]):
            raise AssertionError(f"{what}: incorrect mounted lifecycle descriptor: {descriptor!r}")
    after = environment.digest(data)
    if after != before:
        raise AssertionError(f"{what}: capability discovery or negotiation changed the image")
    return {"availability": observed, "bounds": report["bounds"],
            "lifecycle": lifecycle, "image_unchanged": True}


def _first_boot(uart, data, refs, refs_ids, lineage, epoch, initial_version, seeded_records):
    discovery = _discovery(uart, data, "boot 1")
    uart.command("cd /workspaces/application")
    uart.command("pwd", "\r\n/workspaces/application\r\n")

    before = _snapshot(data)
    _records(before, seeded_records, "seed7 --scratch")
    scratch = _file(before, refs_ids[1])
    if scratch["path"] != f"/workspaces/application/{SCRATCH_NAME}" or scratch["version"] != initial_version \
            or scratch["size"] != 0:
        raise AssertionError("seed7 scratch file differs from the provisioned V7 identity")

    before_key = environment.digest(data)
    legacy_token = _retry_token(uart, SCRATCH_NAME, LEGACY_KEY, lineage, epoch)
    if environment.digest(data) != before_key:
        raise AssertionError("retry-key changed the V7 image")
    legacy = _old_replace(uart, SCRATCH_NAME, initial_version, legacy_token, LEGACY_CONTENT,
                          "legacy tracked commit")
    _check_old_receipt(legacy["receipt"], refs_ids[1], initial_version,
                       legacy["receipt"]["version"], LEGACY_CONTENT, "legacy tracked commit")
    first_snapshot = _snapshot(data)
    _records(first_snapshot, seeded_records + 1, "legacy tracked commit")
    legacy_record = _check_legacy_record(
        first_snapshot, refs_ids[1], epoch, LEGACY_KEY, initial_version,
        legacy["receipt"]["version"], LEGACY_CONTENT, "legacy tracked commit",
    )
    _check_live(first_snapshot, refs_ids[1], legacy["receipt"]["version"], LEGACY_CONTENT,
                "legacy tracked commit")
    if first_snapshot["sequence"] != before["sequence"] + 1:
        raise AssertionError("legacy tracked commit did not publish exactly one V7 generation")

    before_replay = environment.digest(data)
    replay = _old_replace(uart, SCRATCH_NAME, initial_version, legacy_token, LEGACY_CONTENT,
                          "legacy exact retry")
    _check_old_receipt(replay["receipt"], refs_ids[1], initial_version,
                       legacy["receipt"]["version"], LEGACY_CONTENT, "legacy exact retry")
    if replay["receipt"] != legacy["receipt"] or environment.digest(data) != before_replay:
        raise AssertionError("legacy exact retry changed its receipt or the V7 image")

    changed_before = environment.digest(data)
    if len(CHANGED_CONTENT) != len(LEGACY_CONTENT):
        raise AssertionError("changed-byte conflict fixture must keep the original length")
    _error(
        uart.command(_old_replace_command(SCRATCH_NAME, initial_version, legacy_token, CHANGED_CONTENT),
                     "error: IdempotencyConflict"),
        "IdempotencyConflict",
        "legacy retry with changed bytes",
    )
    if environment.digest(data) != changed_before:
        raise AssertionError("changed-byte refusal changed the V7 image")

    uart.command(f"touch {OTHER_NAME}")
    other = stat_file(uart, OTHER_NAME)
    before_target_conflict = environment.digest(data)
    _error(
        uart.command(_old_replace_command(OTHER_NAME, other["version"], legacy_token, LEGACY_CONTENT),
                     "error: IdempotencyConflict"),
        "IdempotencyConflict",
        "legacy retry redirected to another target",
    )
    if environment.digest(data) != before_target_conflict:
        raise AssertionError("changed-target refusal changed the V7 image")

    foreign = _foreign_token(legacy_token)
    _error(uart.command(f"receipt {refs_ids[1]} {foreign}", "error: Lineage"),
           "Lineage", "foreign legacy token")
    missing_token = _retry_token(uart, SCRATCH_NAME, MISSING_KEY, lineage, epoch)
    _error(uart.command(f"receipt {refs_ids[1]} {missing_token}", "error: OutcomeUnknown"),
           "OutcomeUnknown", "missing legacy receipt")

    modern = profile1._profile1_replace(
        uart, refs, legacy["receipt"]["version"], epoch, SHARED_KEY, MODERN_CONTENT
    )
    check_operation(modern["receipt"], refs["workspace"], refs["resource"],
                    legacy["receipt"]["version"], epoch, SHARED_KEY, MODERN_CONTENT)
    modern_snapshot = _snapshot(data)
    _records(modern_snapshot, seeded_records + 2, "profile-1 tracked commit beside legacy history")
    modern_record = profile1._check_profile1_record(
        modern_snapshot, refs_ids, modern["receipt"], MODERN_CONTENT,
        "profile-1 tracked commit beside legacy history",
    )
    profile1._check_live_bytes(
        modern_snapshot, refs_ids[1], int(modern["receipt"]["receipt"]["version"][2:], 16),
        MODERN_CONTENT, "profile-1 tracked commit",
    )
    _check_legacy_record(modern_snapshot, refs_ids[1], epoch, LEGACY_KEY, initial_version,
                         legacy["receipt"]["version"], LEGACY_CONTENT,
                         "legacy snapshot retained beside a modern commit")

    # The profile-1 record has no legacy discriminator. With its key unique in
    # the flat namespace, the original SDK can replay and look it up exactly.
    shared_token = _retry_token(uart, SCRATCH_NAME, SHARED_KEY, lineage, epoch)
    shared_before = environment.digest(data)
    shared_replay = _old_replace(
        uart, SCRATCH_NAME, legacy["receipt"]["version"], shared_token,
        MODERN_CONTENT, "legacy replay of profile-1 record",
    )
    shared_line = _legacy_matches_profile1(
        shared_replay["receipt"], modern["receipt"], refs_ids[1], MODERN_CONTENT,
        "legacy replay of profile-1 record",
    )
    shared_lookup = _old_lookup(uart, refs_ids[1], shared_token,
                                "legacy lookup of profile-1 record")
    if shared_lookup["line"] != shared_line or environment.digest(data) != shared_before:
        raise AssertionError("profile-1 record was not shared unchanged with legacy RECOVERY")

    # Preserve the old flat retry scan's safety rule: modern scoped operations
    # may create a second same-key record in another workspace scope, but the
    # legacy lookup must refuse to guess between the two.
    current = stat_file(uart, SCRATCH_NAME)
    ambiguous_token = _retry_token(uart, SCRATCH_NAME, AMBIGUOUS_KEY, lineage, epoch)
    ambiguous_legacy = _old_replace(
        uart, SCRATCH_NAME, current["version"], ambiguous_token,
        AMBIGUOUS_LEGACY_CONTENT, "legacy record before ambiguous modern key",
    )
    after_legacy_ambiguous = _snapshot(data)
    _records(after_legacy_ambiguous, seeded_records + 3, "legacy side of ambiguous key")
    _check_legacy_record(
        after_legacy_ambiguous, refs_ids[1], epoch, AMBIGUOUS_KEY,
        current["version"], ambiguous_legacy["receipt"]["version"],
        AMBIGUOUS_LEGACY_CONTENT, "legacy side of ambiguous key",
    )
    ambiguous_modern = profile1._profile1_replace(
        uart, refs, ambiguous_legacy["receipt"]["version"], epoch,
        AMBIGUOUS_KEY, AMBIGUOUS_MODERN_CONTENT,
    )
    check_operation(ambiguous_modern["receipt"], refs["workspace"], refs["resource"],
                    ambiguous_legacy["receipt"]["version"], epoch,
                    AMBIGUOUS_KEY, AMBIGUOUS_MODERN_CONTENT)
    final_before_reboot = _snapshot(data)
    _records(final_before_reboot, seeded_records + 4, "two scopes with one flat legacy retry key")
    ambiguous_modern_record = profile1._check_profile1_record(
        final_before_reboot, refs_ids, ambiguous_modern["receipt"],
        AMBIGUOUS_MODERN_CONTENT, "modern side of ambiguous key",
    )
    _check_legacy_record(
        final_before_reboot, refs_ids[1], epoch, AMBIGUOUS_KEY,
        current["version"], ambiguous_legacy["receipt"]["version"],
        AMBIGUOUS_LEGACY_CONTENT, "legacy side of ambiguous key after collision",
    )

    return {
        "discovery": discovery,
        "legacy": {
            "token": legacy_token,
            "receipt": legacy["receipt"],
            "record": legacy_record,
            "replay": replay["receipt"],
            "changed_bytes": "IdempotencyConflict",
            "changed_target": "IdempotencyConflict",
            "foreign_lineage": "Lineage",
            "missing_key": "OutcomeUnknown",
        },
        "shared_profile1": {
            "operation": modern["receipt"],
            "lines": modern["lines"],
            "record": modern_record,
            "legacy_token": shared_token,
            "legacy_replay": shared_replay["receipt"],
            "legacy_lookup": shared_lookup,
            "identical_receipt_line": shared_line,
        },
        "ambiguous": {
            "token": ambiguous_token,
            "legacy_receipt": ambiguous_legacy["receipt"],
            "modern_operation": ambiguous_modern["receipt"],
            "modern_lines": ambiguous_modern["lines"],
            "modern_record": ambiguous_modern_record,
            "records_before_reboot": len(final_before_reboot["records"]),
            "epoch": epoch,
        },
        "lineage": lineage,
        "epoch": epoch,
        "initial_version": initial_version,
        "seeded_records": seeded_records,
        "final_sequence": final_before_reboot["sequence"],
        "final_content": AMBIGUOUS_MODERN_CONTENT.decode("ascii"),
    }


def _check_maintenance(text, prior_epoch, prior_records):
    lines = [line for line in _lines(text) if line.startswith("maintain-v7 ")]
    if len(lines) != 1:
        raise AssertionError(f"maintain-v7 did not return one completion record: {text!r}")
    match = MAINTENANCE.fullmatch(lines[0])
    if not match:
        raise AssertionError(f"maintain-v7 completion output changed: {lines[0]!r}")
    previous, epoch, records = int(match[1], 16), int(match[2], 16), int(match[3])
    if previous != prior_epoch or epoch != prior_epoch + 1 or records != prior_records:
        raise AssertionError(
            f"maintain-v7 returned previous={previous}, epoch={epoch}, records={records}; "
            f"expected {prior_epoch}, {prior_epoch + 1}, {prior_records}"
        )
    return {"previous_epoch": previous, "epoch": epoch, "records": records,
            "sectors": int(match[4]), "job": int(match[5]), "ticks": int(match[6])}


def _second_boot(uart, data, refs, refs_ids, first, preboot_digest):
    before = _snapshot(data)
    _records(before, first["seeded_records"] + 4, "boot-2 retained records")
    if environment.digest(data) != preboot_digest:
        raise AssertionError("boot 2 changed the V7 image before its first command")
    discovery = _discovery(uart, data, "boot 2")
    if environment.digest(data) != preboot_digest:
        raise AssertionError("boot-2 discovery changed the V7 image")
    uart.command("cd /workspaces/application")

    legacy = first["legacy"]
    old_token = legacy["token"]
    cold_by_id = _old_lookup(uart, refs_ids[1], old_token, "cold legacy receipt lookup")
    if cold_by_id != legacy["receipt"]:
        raise AssertionError("cold legacy lookup differs from the commit-time SDK receipt")
    lookup_digest = environment.digest(data)
    replay = _old_replace(
        uart, SCRATCH_NAME, first["initial_version"], old_token,
        LEGACY_CONTENT, "post-reboot legacy exact retry",
    )
    if replay["receipt"] != legacy["receipt"] or environment.digest(data) != lookup_digest:
        raise AssertionError("post-reboot legacy replay changed its receipt or durable image")

    shared = first["shared_profile1"]
    modern = shared["operation"]
    profile1_by_id = profile1._profile1_operation(
        uart, f"operation {modern['operation_id']}",
        {"receipt": modern, "lines": shared["lines"]},
        "cold profile-1 ID lookup",
    )
    profile1_by_retry = profile1._profile1_operation(
        uart,
        f"operation {refs['workspace']} e_{first['epoch']:016x} k_{SHARED_KEY:016x}",
        {"receipt": modern, "lines": shared["lines"]},
        "cold profile-1 retry lookup",
    )
    if profile1_by_retry["lines"] != profile1_by_id["lines"]:
        raise AssertionError("cold profile-1 ID and retry lookups differ")

    shared_token = shared["legacy_token"]
    cold_shared = _old_lookup(uart, refs_ids[1], shared_token,
                              "cold legacy lookup of a modern record")
    if cold_shared["line"] != shared["identical_receipt_line"]:
        raise AssertionError("cold legacy lookup changed the shared profile-1 receipt")

    ambiguous = first["ambiguous"]
    # The flat old API refuses the collision; the scoped modern lookup remains
    # deterministic because its workspace is part of that API's retry key.
    ambiguous_error = _error(
        uart.command(f"receipt {refs_ids[1]} {ambiguous['token']}", "error:"),
        "IdempotencyConflict",
        "ambiguous flat legacy receipt lookup",
    )
    scoped = ambiguous["modern_operation"]
    cold_scoped = profile1._profile1_operation(
        uart,
        f"operation {refs['workspace']} e_{first['epoch']:016x} k_{AMBIGUOUS_KEY:016x}",
        {"receipt": scoped, "lines": ambiguous["modern_lines"]},
        "cold scoped lookup after flat-key collision",
    )
    if cold_scoped["lines"][0] != f"operation-v1 id={scoped['operation_id']} service_instance={scoped['service_instance']} state=succeeded effect=committed cancel_requested=false":
        raise AssertionError("scoped modern lookup selected the wrong side of a flat retry collision")

    before_ambiguous_replace = environment.digest(data)
    _error(
        uart.command(_old_replace_command(
            SCRATCH_NAME,
            ambiguous["legacy_receipt"]["previous"],
            ambiguous["token"],
            AMBIGUOUS_LEGACY_CONTENT,
        ), "error: IdempotencyConflict"),
        "IdempotencyConflict",
        "ambiguous flat legacy retry",
    )
    if environment.digest(data) != before_ambiguous_replace:
        raise AssertionError("ambiguous flat-key refusal changed the V7 image")

    prior = _snapshot(data)
    _records(prior, first["seeded_records"] + 4, "pre-maintenance records")
    maintained = _check_maintenance(
        uart.command("maintain-v7", "maintain-v7 previous="),
        first["epoch"], len(prior["records"]),
    )
    after = _snapshot(data)
    _records(after, 0, "post-maintenance retained records")
    if after["epoch"] != maintained["epoch"] or after["sequence"] <= prior["sequence"]:
        raise AssertionError("V7 maintenance did not publish the new retry epoch")
    current = stat_file(uart, SCRATCH_NAME)
    if not (current["version"] == _file(after, refs_ids[1])["version"]
            == _file(prior, refs_ids[1])["version"]):
        raise AssertionError("V7 maintenance changed the live scratch version")
    _check_live(after, refs_ids[1], current["version"], AMBIGUOUS_MODERN_CONTENT,
                "post-maintenance live scratch")

    expired_before = environment.digest(data)
    expired_lookup = _error(
        uart.command(f"receipt {refs_ids[1]} {old_token}", "error: ExpiredEpoch"),
        "ExpiredEpoch",
        "expired legacy receipt",
    )
    expired_retry = _error(
        uart.command(_old_replace_command(SCRATCH_NAME, current["version"], old_token, LEGACY_CONTENT),
                     "error: ExpiredEpoch"),
        "ExpiredEpoch",
        "expired legacy replacement",
    )
    if environment.digest(data) != expired_before:
        raise AssertionError("expired old-epoch calls changed the V7 image")

    new_epoch_token = _retry_token(uart, SCRATCH_NAME, MISSING_KEY, first["lineage"], maintained["epoch"])
    missing_after_maintenance = _error(
        uart.command(f"receipt {refs_ids[1]} {new_epoch_token}", "error: OutcomeUnknown"),
        "OutcomeUnknown",
        "empty current-epoch receipt lookup",
    )
    if environment.digest(data) != expired_before:
        raise AssertionError("a cold current-epoch miss changed the V7 image")

    return {
        "discovery": discovery,
        "cold_legacy_id_lookup": cold_by_id,
        "post_reboot_legacy_retry": replay["receipt"],
        "cold_profile1_id_lookup": profile1_by_id["lines"],
        "cold_profile1_retry_lookup": profile1_by_retry["lines"],
        "cold_legacy_shared_lookup": cold_shared,
        "flat_key_collision": ambiguous_error,
        "scoped_lookup_after_collision": cold_scoped["lines"],
        "maintenance": maintained,
        "expired_lookup": expired_lookup,
        "expired_replace": expired_retry,
        "current_epoch_miss": missing_after_maintenance,
        "records_before_maintenance": len(prior["records"]),
        "records_after_maintenance": len(after["records"]),
        "final_epoch": after["epoch"],
        "final_sequence": after["sequence"],
    }


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-recovery")
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    elf = ROOT / "target/native/file-server.elf"
    manifest = ROOT / "target/native/file-server.manifest"
    for path, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(path) != metadata["native_applications"]["file-server"][suffix]:
            raise RuntimeError(f"{path.name} differs from the artifact recorded by the boot build")

    (output / "result.json").unlink(missing_ok=True)
    phases = (1, 2)
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-recovery-") as temporary:
            temporary = Path(temporary)
            data = temporary / "v7-recovery-volume.raw"
            lineage = uuid.uuid4().hex
            seeded = volume_json(volume_tool, "seed7", data, lineage, elf, manifest, "--scratch")
            if seeded["lineage"] != lineage:
                raise AssertionError("seed7 created a volume with another lineage")
            initial = _snapshot(data)
            _records(initial, SEEDED_RECORDS, "fresh seed")
            if initial["lineage"] != lineage:
                raise AssertionError("oracle7 found a different initial lineage")
            refs = {"workspace": seeded["workspace"]["text"],
                    "resource": seeded["scratch"]["resource"]}
            refs_ids = (seeded["workspace"]["id"], seeded["scratch"]["id"])
            epoch = initial["epoch"]
            initial_version = seeded["scratch"]["version"]
            first = boot_terminal(
                image, data, output, 1, temporary,
                lambda uart: _first_boot(
                    uart, data, refs, refs_ids, lineage, epoch, initial_version,
                    len(initial["records"]),
                ),
            )
            after_first = _snapshot(data)
            _records(after_first, SEEDED_RECORDS + 4, "boot-1 shutdown")
            if after_first["sequence"] != first["final_sequence"] or after_first["epoch"] != epoch:
                raise AssertionError("boot-1 shutdown changed the selected V7 header state")
            before_second = environment.digest(data)
            report = volume_json(volume_tool, "report7", data)
            if report["lineage"] != lineage or report["sequence"] != after_first["sequence"] \
                    or len(report["records"]) != len(after_first["records"]):
                raise AssertionError("report7 and independent oracle7 disagree after boot 1")

            second = boot_terminal(
                image, data, output, 2, temporary,
                lambda uart: _second_boot(uart, data, refs, refs_ids, first, before_second),
            )
            final = _snapshot(data)
            _records(final, 0, "boot-2 shutdown after maintenance")
            report = volume_json(volume_tool, "report7", data)
            if report["lineage"] != lineage or report["sequence"] != final["sequence"] \
                    or report["epoch"] != final["epoch"] or report["records"]:
                raise AssertionError("report7 and independent oracle7 disagree after maintenance")
            if final["lineage"] != lineage:
                raise AssertionError("reboot or maintenance changed the V7 lineage")

        evidence = {
            "verified": True,
            "mode": "terminal-v7",
            "boots": 2,
            "lineage": lineage,
            "subject": SHELL_SUBJECT,
            "retained_limit": RETAINED,
            "seeded_records": len(initial["records"]),
            "boot_1": first,
            "boot_2": second,
            "final": {
                "epoch": final["epoch"],
                "sequence": final["sequence"],
                "records": len(final["records"]),
                "scratch": _file(final, refs_ids[1]),
                "oracle_sha256": final["files"],
            },
        }
        result = {
            "outcome": "success",
            "returncode": 33,
            "timed_out": False,
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "build_id": metadata["build_id"],
            "image_sha256": metadata["image_sha256"],
            "terminal_v7_recovery": evidence,
        }
        (output / "recovery-v7.json").write_text(json.dumps(evidence, separators=(",", ":")) + "\n")
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print("V7 recovery acceptance: old tracked receipts, exact replays, reboot lookups, shared profile-1 history, "
              "ambiguity refusals and epoch expiry matched oracle7 across two boots.", flush=True)
        return result
    finally:
        for kind in ("serial", "qemu"):
            (output / f"{kind}.log").write_bytes(b"\n".join(
                (output / f"{kind}-{phase}.log").read_bytes()
                for phase in phases if (output / f"{kind}-{phase}.log").exists()
            ))
