# SPDX-License-Identifier: Apache-2.0
"""Default manual V7 authority, recovery and persistence on the R0 device."""
import json
import os
from pathlib import Path
import re
import tempfile
import time

import environment
from . import tasks_owner, v7_disk
from .cases import counters, exited, pid
from .operation_cases import references
from .recovery_cases import key, stat
from .v7_plain import metadata
from .v7_write import boot_terminal, command, decode_cut


def snapshot(data):
    fd = os.open(data, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        return v7_disk.snapshot(fd)
    finally:
        os.close(fd)


def first(uart, data):
    baseline = counters(uart)
    for root in ("system", "data", "config", "workspaces"):
        uart.command(f"ls /{root}")
        uart.command(f"stat /{root}", "kind=directory")
    uart.command("cat /config/owner-policy", "helpers=explicit")
    uart.command("write /system/refused forbidden", "error: ReadOnly")
    uart.command("touch /new-root", "error: Denied")
    uart.command("write /data/manual-note data persists")
    uart.command("write /config/manual-note config persists")
    uart.command("write lost-target before")
    uart.command("write lost-other private")
    before = stat(uart, "lost-target")
    token = key(uart, 77, "lost-target")
    child = pid(uart, "run lost-reply lost-target lost-other")
    exited(uart, child, 1, 0)
    uart.command(f"reap {child}", "code=0")
    uart.command(f'receipt {before["id"]} {token}', "committed id=")
    assert counters(uart) == baseline

    # The shell owns its /config journal under the ordinary manual policy.
    tasks_owner.write_document(uart, "tasks-doc", tasks_owner.DOCUMENT)
    uart.command("tasks enable", "mounted format v7")
    uart.command('tasks add tasks-doc "Manual V7"')
    uart.command("tasks list tasks-doc", "Manual V7")
    state = snapshot(data)
    assert state["contents"]["/workspaces/tasks-doc"] == tasks_owner.added(
        tasks_owner.DOCUMENT, "Manual V7").encode("ascii")
    assert state["contents"]["/config/tasks-intent"] == b""
    assert {record["subject"] for record in state["records"]} == {1}

    # Revocation remount phases must preserve Manual, rather than substitute
    # the dedicated workspace fixture's scope and retry subject.
    workspace, resource = references(uart, "/data", "/data/manual-note")
    target = metadata(uart, "/data/manual-note")
    cut = decode_cut(uart.command(command(workspace, resource, target["version"],
                                        state["epoch"], 99, 8, 1024) + " cut 1"))
    assert cut["old"] == "Closed" and cut["new"] == "NoTransfer"
    uart.command("cat /data/manual-note", "data persists")
    uart.command("cat /config/manual-note", "config persists")
    uart.command(f'receipt {before["id"]} {token}', "committed id=")
    uart.command("restart files", "utility sessions revoked")
    uart.command("cat /data/manual-note", "data persists")
    uart.command("cat /config/manual-note", "config persists")
    uart.command(f'receipt {before["id"]} {token}', "committed id=")
    assert counters(uart) == baseline
    return {"id": before["id"], "token": token, "subject": 1,
            "whole_volume": True, "shell_journal": True, "rebind": cut,
            "restart_policy_preserved": True}


def identities(initialize_image, mount_image):
    images = tuple(Path(path).resolve() for path in (initialize_image, mount_image))
    metadata = tuple(json.loads((path.parent / "image.json").read_text()) for path in images)
    for image, identity, mode in zip(images, metadata, ("terminal-init", "terminal")):
        if (identity.get("mode") != mode or identity.get("tasks_acceptance") is not False
                or not re.fullmatch(r"[0-9a-f]{16}", str(identity.get("build_id", "")))
                or environment.digest(image) != identity.get("image_sha256")
                or environment.digest(image.parent / "kernel.elf") != identity.get("kernel_sha256")):
            raise ValueError("manual acceptance requires the verified ordinary default boot images")
    if (metadata[0].get("build_id") != metadata[1].get("build_id")
            or metadata[0]["kernel_sha256"] != metadata[1]["kernel_sha256"]):
        raise ValueError("manual acceptance must reboot the same build and kernel")
    return metadata


def verify(initialize_image, mount_image, volume_tool, output=None):
    identity, mounted = identities(initialize_image, mount_image)
    output = Path(output or environment.ROOT / "artifacts/boot/terminal-v7-manual")
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").unlink(missing_ok=True)
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="rustic-v7-manual-") as temporary:
        temporary = Path(temporary)
        path = temporary / "data.raw"
        with v7_disk.disk(path, True, volume_tool=volume_tool) as data:
            assert data.stat().st_size == v7_disk.SIZE
            one = boot_terminal(initialize_image, data, output, 1, temporary,
                                lambda uart: first(uart, data))
        persisted = snapshot(path)
        digest = environment.digest(path)

        def second(uart):
            uart.command("cat /data/manual-note", "data persists")
            uart.command("cat /config/manual-note", "config persists")
            uart.command("cat /config/owner-policy", "helpers=explicit")
            uart.command(f'receipt {one["id"]} {one["token"]}', "committed id=")
            uart.command("tasks list tasks-doc", "Manual V7")
            uart.command("tasks recover")
            return {"same_subject_receipt": True, "task_recovery_idle": True}

        with v7_disk.disk(path) as data:
            two = boot_terminal(mount_image, data, output, 2, temporary, second)
        assert snapshot(path) == persisted and environment.digest(path) == digest
    result = {"outcome": "success", "returncode": 33, "timed_out": False,
              "build_id": identity["build_id"], "image_sha256": identity["image_sha256"],
              "mount_image_sha256": mounted["image_sha256"], "kernel_sha256": identity["kernel_sha256"],
              "elapsed_seconds": round(time.monotonic() - started, 3),
              "terminal_v7_manual": {"verified": True, "boots": 2,
                                     "device_bytes": v7_disk.SIZE,
                                     "filesystem_bytes": v7_disk.PREFIX_BYTES,
                                     "boot_modes": [identity["mode"], mounted["mode"]],
                                     "first": one, "second": two,
                                     "entire_device_unchanged_after_reboot": True}}
    (output / "result.json").write_text(json.dumps(result, separators=(",", ":")) + "\n")
    print("Default V7 terminal: whole-volume manual authority, shell tasks, same-subject "
          "recovery, restart/rebind and unchanged reboot verified.", flush=True)
    return result
