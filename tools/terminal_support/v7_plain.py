# SPDX-License-Identifier: Apache-2.0
"""Ordinary namespace mutations and untracked writes through the native shell."""
import hashlib
import json
from pathlib import Path
import re
import tempfile
import time
import uuid

import environment
from . import oracle7
from .v7_read import volume_json
from .v7_write import boot_terminal


ROOT = environment.ROOT
DIRECTORY = "/workspaces/manual"
FILE = DIRECTORY + "/note.txt"
CONTENT = "ordinary V7 write 19"


def metadata(uart, path):
    text = uart.command(f"stat {path}")
    found = re.findall(r"(?m)^id=(\d+) parent=(\d+) kind=(file|directory) bytes=(\d+) version=(\d+)\r?$", text)
    if len(found) != 1:
        raise AssertionError(f"ambiguous native metadata for {path}: {text!r}")
    identifier, parent, kind, size, version = found[0]
    return {"id": int(identifier), "parent": int(parent), "kind": kind,
            "size": int(size), "version": int(version)}


def first_boot(uart):
    uart.command(f"mkdir {DIRECTORY}")
    directory = metadata(uart, DIRECTORY)
    uart.command(f"touch {FILE}")
    previous = metadata(uart, FILE)
    uart.command(f"cat {FILE}")
    writes = []
    for index in range(20):
        content = f"ordinary V7 write {index}"
        uart.command(f"write {FILE} {content}", f"written {len(content)} bytes version=")
        current = metadata(uart, FILE)
        if (current["id"], current["parent"], current["kind"], current["size"]) != (
                previous["id"], directory["id"], "file", len(content)) or current["version"] <= previous["version"]:
            raise AssertionError("ordinary replacement did not preserve identity and advance its exact version")
        uart.command(f"cat {FILE}", "\r\n" + content + "\r\n")
        writes.append(current["version"])
        previous = current
    uart.command(f"rm {DIRECTORY}", "error: NotEmpty")
    uart.command(f"mkdir {DIRECTORY}", "error: Exists")
    uart.command(f"touch {FILE}", "error: Exists")
    uart.command(f"cat {DIRECTORY}", "error: IsDirectory")

    disposable = DIRECTORY + "/removed.txt"
    uart.command(f"write {disposable} remove these bytes")
    removed = metadata(uart, disposable)
    uart.command(f"rm {disposable}")
    uart.command(f"stat {disposable}", "error: NotFound")
    uart.command(f"touch {disposable}")
    replacement = metadata(uart, disposable)
    if replacement["id"] <= removed["id"]:
        raise AssertionError("removed object identity was reused")
    uart.command(f"rm {disposable}")
    uart.command(f"mkdir {DIRECTORY}/empty")
    uart.command(f"rm {DIRECTORY}/empty")
    uart.command(f"stat {DIRECTORY}/empty", "error: NotFound")
    uart.command("touch /new-root", "error: Denied")
    return {"directory": directory, "file": previous, "write_versions": writes,
            "removed_id": removed["id"], "replacement_id": replacement["id"]}


def check_snapshot(snapshot, initial, first, elf, manifest):
    if snapshot["records"] != initial["records"] or snapshot["epoch"] != initial["epoch"]:
        raise AssertionError("ordinary file work changed retained outcomes or the retry epoch")
    expected = CONTENT.encode("ascii")
    if snapshot["contents"].get(FILE) != expected:
        raise AssertionError("independent V7 reader found different persisted manual bytes")
    facts = first["file"]
    live = next((file for file in snapshot["files"] if file["id"] == facts["id"]), None)
    if live is None or (live["size"], live["version"], live["sha256"]) != (
            facts["size"], facts["version"], hashlib.sha256(expected).hexdigest()):
        raise AssertionError("independent persisted file identity/version/hash differs from the terminal")
    for source in (elf, manifest):
        path = "/workspaces/application/" + source.name
        if snapshot["contents"].get(path) != source.read_bytes():
            raise AssertionError(f"ordinary file work changed the provisioned {source.name}")
    paths = {node["path"] for node in snapshot["nodes"].values()}
    if paths.intersection((DIRECTORY + "/removed.txt", DIRECTORY + "/empty")):
        raise AssertionError("removed paths remain in the independent V7 namespace")


def verify(image, volume_tool, output=None):
    image, volume_tool = Path(image).resolve(), Path(volume_tool).resolve()
    output = Path(output or ROOT / "artifacts/boot/terminal-v7-plain")
    output.mkdir(parents=True, exist_ok=True)
    metadata_image = json.loads((image.parent / "image.json").read_text())
    elf, manifest = ROOT / "target/native/file-server.elf", ROOT / "target/native/file-server.manifest"
    for source, suffix in ((elf, ".elf"), (manifest, ".manifest")):
        if environment.digest(source) != metadata_image["native_applications"]["file-server"][suffix]:
            raise RuntimeError("native artifact differs from the boot image identity")
    (output / "result.json").unlink(missing_ok=True)
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-v7-plain-") as temporary:
            temporary = Path(temporary)
            data = temporary / "volume.raw"
            lineage = uuid.uuid4().hex
            volume_json(volume_tool, "seed7", data, lineage, elf, manifest)
            initial = oracle7.snapshot(data.read_bytes())
            if len(initial["records"]) != 2:
                raise AssertionError("seed must retain the ELF and manifest snapshots")
            first = boot_terminal(image, data, output, 1, temporary, first_boot)
            snapshot = oracle7.snapshot(data.read_bytes())
            check_snapshot(snapshot, initial, first, elf, manifest)
            before_reboot = environment.digest(data)

            def second_boot(uart):
                current = metadata(uart, FILE)
                if current != first["file"]:
                    raise AssertionError("reboot changed persisted manual metadata")
                uart.command(f"cat {FILE}", "\r\n" + CONTENT + "\r\n")
                uart.command("restart files", "utility sessions revoked")
                after_restart = metadata(uart, FILE)
                if after_restart != current:
                    raise AssertionError("service restart changed persisted manual metadata")
                uart.command(f"cat {FILE}", "\r\n" + CONTENT + "\r\n")
                return {"file": current, "service_restart": True}

            second = boot_terminal(image, data, output, 2, temporary, second_boot)
            after_reboot = environment.digest(data)
            if before_reboot != after_reboot:
                raise AssertionError("read-only reboot and service restart changed the V7 volume")
            final = oracle7.snapshot(data.read_bytes())
            check_snapshot(final, initial, first, elf, manifest)
            report = volume_json(volume_tool, "report7", data)
            if report["sequence"] != final["sequence"] or len(report["records"]) != len(initial["records"]):
                raise AssertionError("Rust remount and independent reader disagree")
        result = {"outcome": "success", "returncode": 33, "timed_out": False,
                  "elapsed_seconds": round(time.monotonic() - started, 3),
                  "build_id": metadata_image["build_id"], "image_sha256": metadata_image["image_sha256"],
                  "terminal_v7_plain": {"verified": True, "boots": 2, "lineage": lineage,
                                        "first": first, "second": second, "retained_records_unchanged": True,
                                        "read_only_reboot_volume_unchanged": True,
                                        "content_sha256": hashlib.sha256(CONTENT.encode("ascii")).hexdigest()}}
        (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
        print("V7 ordinary terminal acceptance: namespace mutations, 20 untracked writes, unchanged retained "
              "outcomes, deletion without identity reuse, exact persisted readback across reboot and service restart.",
              flush=True)
        return result
    finally:
        for kind, extension in (("serial", "log"), ("qemu", "log")):
            (output / f"{kind}.{extension}").write_bytes(b"\n".join(
                (output / f"{kind}-{phase}.{extension}").read_bytes()
                for phase in (1, 2) if (output / f"{kind}-{phase}.{extension}").exists()))
