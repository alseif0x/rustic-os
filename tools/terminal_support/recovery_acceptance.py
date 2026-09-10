# SPDX-License-Identifier: Apache-2.0
"""Native recovery on real VirtIO storage, independent oracle and selected QEMU I/O faults."""
import contextlib
import json
from pathlib import Path
import struct
import tempfile
import time
import zlib
from boot_support.image import package
from .machine import machine, disk, SIZE
from .connection import Connection
from .failure import preserve_failure
from .oracle import snapshot
from .recovery_cases import exercise, verify_final
from .recovery_faults import CUTS
from .cases import pid
from .authority_cases import actor, fence


def verify(image, timeout=60, output=None):
    image = Path(image).resolve()
    output = Path(output or image.parent)
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / "image.json").read_text())
    mount = package(image.parent / "kernel.elf", "terminal", metadata["build_id"], {})
    serials, logs, cases = [], [], []
    started = time.monotonic()
    @contextlib.contextmanager
    def owned_disk(path, initialize=False, upgrade_recovery=False, *, evidence_name):
        # The independent oracle also runs inside the disk lifetime. Sessions
        # stop their VMs before this outer scope captures a failed oracle.
        with disk(path, initialize, upgrade_recovery) as data:
            with preserve_failure(data, output, evidence_name, metadata):
                yield data
    @contextlib.contextmanager
    def session(boot, data, name, fault=None):
        transcript, log = output / (name + ".serial.log"), output / (name + ".qemu.log")
        serials.append(transcript); logs.append(log)
        with preserve_failure(data, output, name, metadata), tempfile.TemporaryDirectory(prefix="rustic-recovery-uart-") as socket_dir:
            sock = Path(socket_dir) / "uart.sock"
            with machine(boot, data, f"unix:{sock},server=on,wait=off", log, fault) as vm:
                uart = Connection(sock, vm, transcript, timeout, output / (name + ".commands.jsonl"))
                try:
                    uart.until()
                    yield uart
                    if fault is None:
                        uart.send(b"exit\r")
                        uart.until(b"RUSTIC TERMINAL stopped=1 reclaimed=1")
                        if vm.wait(timeout=10) != 33:
                            raise AssertionError("unclean recovery VM exit")
                    else:
                        # Abruptly terminate only this owned VM after the guest has observed the I/O failure.
                        vm.kill(); vm.wait(timeout=10)
                finally:
                    uart.close()
    try:
        with tempfile.TemporaryDirectory(prefix="rustic-recovery-test-") as temporary:
            temporary = Path(temporary)
            with owned_disk(temporary / "data.raw", True, evidence_name="reboot") as data:
                base = {}
                def capture(old, retry):
                    base.update(old=old, retry=retry, bytes=snapshot(data)[0])
                with session(image, data, "initial") as uart:
                    final = exercise(uart, capture)
                with session(mount, data, "reboot") as uart:
                    verify_final(uart, final)
                selected, state = snapshot(data)
                assert state["files"][(4,"hello")] == b"after"
                assert len(state["records"]) == 1 and state["records"][0]["content"] == b"after"
                record = state["records"][0]
                assert state["nodes"][record["id"]]["version"] == record["committed"]
                assert state["records"][0]["epoch"] == state["epoch"] == 2
                cases.append({"case":"lost_reply_replay_conflict_quota_rotation_reboot", "verified":True, "sha256":state["selected_sha256"]})
            for name, cut in CUTS.items():
                with owned_disk(temporary / ("cut-" + name + ".raw"), True, evidence_name=name + "-reboot") as data:
                    with data.open("r+b") as f: f.write(base["bytes"])
                    with session(mount, data, name + "-fault", cut) as uart:
                        c = pid(uart, "session hello other")
                        h = pid(uart, f"helper {c} hello other")
                        uart.command(f'replace hello {base["old"]["version"]} {base["retry"]} "after"', "Uncertain")
                        fence(uart, c, "access=fenced members=2 discarded_staging=0 effects=recovery-required")
                        actor(uart, h, "read", 18)
                        uart.command("mem", "pending_io=0")
                        uart.command("restart files", "utility sessions revoked")
                        _, observed = snapshot(data)
                        committed = bool(observed["records"])
                        uart.command("cat hello", "after" if committed else "before")
                        uart.command(f'receipt {base["old"]["id"]} {base["retry"]}', "committed id=" if committed else "OutcomeUnknown")
                    with session(mount, data, name + "-reboot") as uart:
                        uart.command("cat hello", "after" if committed else "before")
                        uart.command(f'receipt {base["old"]["id"]} {base["retry"]}', "committed id=" if committed else "OutcomeUnknown")
                    prefix, observed = snapshot(data)
                    assert observed["files"][(4,"other")] == b"untouched"
                    assert observed["files"][(4,"hello")] == (b"after" if committed else b"before")
                    if committed:
                        assert len(observed["records"]) == 1 and observed["records"][0]["content"] == b"after"
                        record = observed["records"][0]
                        assert observed["nodes"][record["id"]]["version"] == record["committed"]
                    else:
                        assert observed["nodes"][base["old"]["id"]]["version"] == base["old"]["version"]
                    (output / (name + ".bin")).write_bytes(prefix)
                    cases.append({"case":name,"cut":cut,"committed":committed,"verified":True,"revocation_reports_recovery_required":True,"sha256":observed["selected_sha256"]})
            from .inflight_cases import exercise as inflight_exercise
            cases.extend(inflight_exercise(session,mount,temporary,base,output,owned_disk))
            # Derive the old format from an independently inspected native volume; preserve its file bytes.
            legacy = bytearray(base["bytes"])
            legacy[512:1024] = bytes(512)
            legacy[160*512:] = bytes(14*512)
            for sector in (8,13):
                h = bytearray(legacy[sector*512:(sector+1)*512])
                h[8] = 1; h[28:36] = bytes(8)
                struct.pack_into("<I",h,28,zlib.crc32(h))
                legacy[sector*512:(sector+1)*512] = h
            path = temporary / "legacy.raw"
            with owned_disk(path, True, evidence_name="legacy") as data:
                with data.open("r+b") as f: f.write(legacy)
                with session(mount, data, "legacy") as uart:
                    uart.command("cat hello", "before")
                    uart.command("retry-key hello 1", "Unsupported")
            with owned_disk(path, upgrade_recovery=True, evidence_name="upgrade") as data:
                with session(mount, data, "upgrade") as uart:
                    uart.command("cat hello", "before")
                    uart.command("retry-key hello 1", "retry-key=")
                assert path.with_name(path.name + ".pre-recovery.bin").read_bytes() == legacy
                _, upgraded = snapshot(data)
                assert upgraded["files"][(4,"hello")] == b"before" and upgraded["epoch"] == 1
                assert not upgraded["records"]
                cases.append({"case":"legacy_mount_and_explicit_upgrade", "verified":True,"sha256":upgraded["selected_sha256"]})
            from .operation_cases import verify as verify_operations
            operation_cases, selected = verify_operations(session, owned_disk, temporary, image, mount)
            cases.extend(operation_cases)
        (output / "files.bin").write_bytes(selected)
        evidence = {"verified":True,"boots":len(serials),"cases":cases,"kernel_sha256":metadata["kernel_sha256"],"build_id":metadata["build_id"]}
        (output / "recovery.json").write_text(json.dumps(evidence,indent=2)+"\n")
        result = {"outcome":"success","returncode":33,"timed_out":False,"elapsed_seconds":round(time.monotonic()-started,3),"build_id":metadata["build_id"],"image_sha256":metadata["image_sha256"],"recovery":evidence}
        (output / "result.json").write_text(json.dumps(result,indent=2)+"\n")
        print(f"Native recovery: {len(cases)} cases, {len(serials)} VM boots; real I/O faults, lost reply and independent receipt/content oracle.",flush=True)
        return result
    finally:
        (output / "serial.log").write_bytes(b"\n".join(p.read_bytes() for p in serials if p.exists()))
        (output / "qemu.log").write_bytes(b"\n".join(p.read_bytes() for p in logs if p.exists()))
