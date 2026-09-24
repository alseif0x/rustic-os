# SPDX-License-Identifier: Apache-2.0
"""Executable rollback on one migrated V7 volume, separate from data migration (#51, #52).

Data migration and executable rollback are different operations here:

* Data migration is a one-way host step. `rustic-volume seed5-history` builds a
  disposable v5 source and `migrate7` copies it into a new V7 image; the source
  is never given to QEMU and its SHA-256 must not change.
* Publishing executables is a second host step on the V7 image: `add7` places
  the tag-1 utility pair and then the tag-2 pair beside the migrated history,
  so tag 1 holds the older V7 file versions and tag 2 the newer ones.
* Executable rollback is selection only. In ONE boot of the frozen
  `terminal-v7` image the owner stages and starts the tag-2 pair (report tag 2,
  exit 7, reap), then stages and starts the tag-1 pair by its older pins
  (report tag 1). Nothing is written: the volume digest, `report7` and the
  `oracle7` view (files, bytes and retained records, including the migrated
  ones) are identical before and after the boot.

There is no persistent "current version": the owner's pins select the pair for
each stage. Both variants carry the same manifest identity and version; the
"older" pair is the one published first, by V7 file version. Pins and SHA-256
bind a pair; they do not authenticate a publisher.
"""
import json
from pathlib import Path
import tempfile
import time
import uuid

import environment
from . import oracle7
from .connection import Connection
from .machine import machine
from .v7_launch import BOOT_TIMEOUT, TAGS, build_variant, finish_case, freeze
from .v7_migration import check_migrated
from .v7_read import version_text, volume_json


ROOT = environment.ROOT
# Publication order: the first pair published is the older one.
PUBLISHED = (1, 2)
# Boot order: the newer pair runs first, then the owner rolls back to the older.
STARTED = (2, 1)
WORKSPACE = "/workspaces/migrated"


def publish(volume_tool, target, variant):
    """`add7` one tagged ELF/manifest pair and check the tool's answer against the build."""
    tag = variant["tag"]
    answers = {}
    for kind, source, digest in (("elf", variant["elf"], variant["elf_sha256"]),
                                 ("manifest", variant["manifest"], variant["manifest_sha256"])):
        name = f"utility-tag-{tag}.{kind}"
        answer = volume_json(volume_tool, "add7", target, WORKSPACE, name, source)
        file = answer["file"]
        if (file["name"], file["size"], file["sha256"], answer["record"]["state"], answer["record"]["committed"]) \
                != (name, Path(source).stat().st_size, digest, "direct_committed", file["version"]):
            raise AssertionError(f"add7 published another file for {name}: {answer}")
        answers[kind] = answer
    if answers["elf"]["workspace"] != answers["manifest"]["workspace"]:
        raise AssertionError("the pair was published into two workspaces")
    return answers


def pin(published):
    """The owner's `stage-ref` arguments for one published pair."""
    elf, manifest = published["elf"], published["manifest"]
    return (elf["workspace"]["text"], elf["file"]["resource"], manifest["file"]["resource"],
            version_text(elf["file"]["version"]), version_text(manifest["file"]["version"]))


def check_order(published):
    """Every file of the tag-1 pair must be older (lower V7 version) than every tag-2 file."""
    older = [published[1][kind]["file"]["version"] for kind in ("elf", "manifest")]
    newer = [published[2][kind]["file"]["version"] for kind in ("elf", "manifest")]
    if max(older) >= min(newer):
        raise AssertionError(f"tag 1 is not the older pair: {older} vs {newer}")


def check_selection(finishes, pins):
    """Each start must have reported its own tag from the versions the owner pinned."""
    for tag, finish in finishes:
        wanted = {"elf": pins[tag][3], "manifest": pins[tag][4]}
        if finish["report"]["bytes"] != tag or finish["staged_versions"] != wanted:
            raise AssertionError(f"start {tag} ran another pair: {finish['report']} {finish['staged_versions']}")
    if [tag for tag, _ in finishes] != list(STARTED):
        raise AssertionError(f"the pairs did not start newer first, then older: {finishes}")


def unchanged(before, after):
    """The fields of two `oracle7` snapshots a read-only boot must leave identical."""
    keys = ("lineage", "generation", "recovered", "sequence", "epoch", "next", "nodes", "files", "contents",
            "records", "free_sectors")
    return [key for key in keys if before[key] != after[key]]


def _boot(image, data, pins, temporary, output):
    sock = temporary / "uart-rollback.sock"
    image_sha256 = environment.digest(image)
    with machine(image, data, f"unix:{sock},server=on,wait=off", output / "qemu.log") as vm:
        uart = Connection(sock, vm, output / "serial.log", BOOT_TIMEOUT, output / "commands.jsonl")
        try:
            uart.until()
            uart.send(b"job-status 1\r")
            mount = uart.until()
            if "status=0" not in mount:
                raise AssertionError(f"initial V7 mount job did not succeed: {mount!r}")
            finishes = [(tag, finish_case(uart, pins[tag], tag)) for tag in STARTED]
            uart.send(b"exit\r")
            uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
            if vm.wait(timeout=10) != 33:
                raise RuntimeError("unclean terminal-v7 exit")
        finally:
            uart.close()
    return image_sha256, finishes


def verify(image, volume_tool, output=None):
    image = Path(image).resolve()
    volume_tool = Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-rollback")
    output.mkdir(parents=True, exist_ok=True)
    for name in ("result.json", "terminal-v7-rollback.json"):
        (output / name).unlink(missing_ok=True)
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="rustic-terminal-v7-rollback-") as temporary:
        temporary = Path(temporary)
        frozen, metadata, kernel_in_image = freeze(image, temporary / "image")
        variants = {variant["tag"]: variant for variant in (build_variant(tag) for tag in TAGS)}
        if variants[1]["elf_sha256"] == variants[2]["elf_sha256"]:
            raise AssertionError("the two utility variants are the same ELF")

        # Data migration: one-way, out of place, source never booted.
        lineage = uuid.uuid4().hex
        source, target = temporary / "receipts.v5", temporary / "rollback.v7"
        seeded = volume_json(volume_tool, "seed5-history", source, lineage, "receipts")
        source_sha256 = environment.digest(source)
        migrated = volume_json(volume_tool, "migrate7", source, target, lineage)
        migrated_view = oracle7.snapshot(target.read_bytes())
        check_migrated(seeded, migrated, migrated_view, source_sha256)

        # Executable publication: tag 1 first (older), then tag 2 (newer).
        published = {tag: publish(volume_tool, target, variants[tag]) for tag in PUBLISHED}
        check_order(published)
        pins = {tag: pin(published[tag]) for tag in TAGS}
        if {pins[tag][0] for tag in TAGS} != {seeded["workspace"]["text"]}:
            raise AssertionError("the pairs were not published into the migrated workspace")
        before = oracle7.snapshot(target.read_bytes())
        if before["records"][:len(migrated_view["records"])] != migrated_view["records"]:
            raise AssertionError("publishing the executables changed a migrated record")
        report_before = volume_json(volume_tool, "report7", target)
        volume_before = environment.digest(target)

        # Executable rollback: one boot, newer pair then older pair, nothing written.
        image_sha256, finishes = _boot(frozen, target, pins, temporary, output)
        check_selection(finishes, pins)
        after = oracle7.snapshot(target.read_bytes())
        volume_after = environment.digest(target)
        changed = unchanged(before, after)
        if volume_after != volume_before or changed or report_before != volume_json(volume_tool, "report7", target):
            raise AssertionError(f"the rollback boot changed the V7 volume: {changed}")
        if environment.digest(source) != source_sha256:
            raise AssertionError("the v5 migration source changed")
        if (image_sha256, environment.digest(frozen)) != (metadata["image_sha256"],) * 2:
            raise AssertionError("the boot did not use the recorded image bytes")
        target_bytes = target.stat().st_size

    evidence = {
        "verified": True,
        "mode": "terminal-v7-rollback",
        "topology": "control-only",
        "meaning": {
            "data_migration": "one-way host step: seed5-history source -> migrate7 into a new V7 image; the "
                              "source is never booted and its SHA-256 is unchanged",
            "executable_rollback": "in one boot of the unchanged kernel image, the owner pins and starts the "
                                   "older tag-1 pair after the newer tag-2 pair on an unchanged volume",
            "not_claimed": ["a persistent current-version pointer (selection is owner-pinned per stage)",
                            "publisher authentication (SHA-256 binds the pair, it does not authenticate)",
                            "a semantic manifest version difference (both variants declare the same version)"],
        },
        "image": {
            "build_id": metadata["build_id"],
            "source_commit": metadata["source_commit"],
            "source_status_clean": metadata["source_status"] == "",
            "image_sha256": metadata["image_sha256"],
            "kernel_sha256": metadata["kernel_sha256"],
            "kernel_in_image_sha256": kernel_in_image,
            "boot_image_sha256": image_sha256,
            "boots": 1,
        },
        "migration": {
            "lineage": lineage,
            "set": seeded["set"],
            "source_sha256_before": migrated["source_sha256_before"],
            "source_sha256_after": migrated["source_sha256_after"],
            "source_sha256_after_boot": source_sha256,
            "migrated_records": [{key: record[key] for key in ("slot", "state", "subject", "object", "key",
                                                               "committed", "sha256")}
                                 for record in migrated_view["records"]],
        },
        "variants": [{key: value for key, value in variants[tag].items() if key not in ("elf", "manifest")}
                     for tag in TAGS],
        "published": {str(tag): {kind: {"id": published[tag][kind]["file"]["id"],
                                        "name": published[tag][kind]["file"]["name"],
                                        "version": published[tag][kind]["file"]["version"],
                                        "sha256": published[tag][kind]["file"]["sha256"],
                                        "record_slot": published[tag][kind]["record"]["slot"]}
                                 for kind in ("elf", "manifest")}
                      for tag in PUBLISHED},
        "volume": {"bytes": target_bytes, "sha256_before": volume_before, "sha256_after": volume_after,
                   "sequence": before["sequence"], "generation": before["generation"],
                   "records": len(before["records"]), "oracle7_unchanged": True, "report7_unchanged": True},
        "starts": [{"tag": tag, **finish} for tag, finish in finishes],
    }
    (output / "terminal-v7-rollback.json").write_text(json.dumps(evidence, indent=1) + "\n")
    result = {
        "outcome": "success",
        "returncode": 33,
        "timed_out": False,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "build_id": metadata["build_id"],
        "image_sha256": metadata["image_sha256"],
        "kernel_sha256": metadata["kernel_sha256"],
        "terminal_v7_rollback": evidence,
    }
    (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
    print(f"V7 rollback acceptance: one boot of image sha256={metadata['image_sha256']} kernel "
          f"sha256={metadata['kernel_sha256']} on migrated volume sha256={volume_after} (unchanged); "
          f"v5 source sha256={source_sha256} (unchanged).", flush=True)
    for tag, finish in finishes:
        print(f"  start tag={tag} elf={finish['staged_versions']['elf']} pid={finish['pid']} "
              f"report={finish['report']['report']} tag_word={finish['report']['bytes']} "
              f"reap code={finish['reaped']['code']}", flush=True)
    return result
