# SPDX-License-Identifier: Apache-2.0
"""Own the guest and a separate paused QEMU exporting only its dedicated test disk."""
import contextlib
import json
import shutil
import subprocess

import environment
from .qmp import Monitor


def _stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=3)


@contextlib.contextmanager
def running(image, data, temporary, output, lifecycle):
    """Sockets and sparse disk are private; no TCP listener, user disk or NIC."""
    variables = temporary / "OVMF_VARS.fd"
    shutil.copyfile("/usr/share/OVMF/OVMF_VARS_4M.fd", variables)
    rules = temporary / "blkdebug.conf"
    rules.write_text("")
    qmp_path, nbd_path, uart_path = (temporary / name for name in
                                      ("backend.sock", "nbd.sock", "uart.sock"))
    backend_command = [
        "stdbuf", "-oL", "-eL", "qemu-system-x86_64", "-machine", "none", "-m", "16M", "-S",
        "-nodefaults", "-display", "none", "-nic", "none",
        "-drive", f"if=none,id=rusticdata,format=raw,cache=writeback,file=blkdebug:{rules}:{data}",
        "-qmp", f"unix:{qmp_path},server=on,wait=off", "-monitor", "none",
    ]
    config = environment.CONFIG
    guest_command = [
        "qemu-system-x86_64", "-machine", config["machine"], "-accel", config["accelerator"],
        "-cpu", config["cpu"], "-smp", "1", "-m", "256M",
        "-drive", "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd",
        "-drive", f"if=pflash,format=raw,file={variables}",
        "-drive", f"if=virtio,format=raw,readonly=on,file={image}",
        "-drive", f"if=none,id=rusticdata,format=raw,cache=writeback,file=nbd:unix:{nbd_path}:exportname=data",
        "-device", "virtio-blk-pci,drive=rusticdata,addr=0x6,disable-modern=on,disable-legacy=off,queue-size=8,num-queues=1,vectors=0,rerror=report,werror=report",
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04", "-display", "none",
        "-serial", f"unix:{uart_path},server=on,wait=off", "-monitor", "none", "-nic", "none", "-no-reboot",
    ]
    for name, command in (("backend", backend_command), ("qemu", guest_command)):
        (output / (name + "-command.json")).write_text(json.dumps(command, indent=2) + "\n")
    backend = guest = monitor = None
    try:
        with (output / "backend.log").open("wb") as log:
            backend = subprocess.Popen(backend_command, stdout=log, stderr=log)
        monitor = Monitor(qmp_path, backend, output / "qmp.jsonl")
        monitor.call("nbd-server-start", {"addr": {"type": "unix", "data": {"path": str(nbd_path)}}})
        monitor.call("nbd-server-add", {"device": "rusticdata", "name": "data", "writable": True})
        with (output / "qemu.log").open("wb") as log:
            guest = subprocess.Popen(guest_command, stdout=log, stderr=log)
        yield guest, monitor, uart_path
    finally:
        # Kill the owned guest first if reset is draining an unreleased backend
        # operation. Separate processes keep backend QMP responsive in that case.
        try:
            _stop(guest)
        finally:
            try:
                _stop(backend)
            finally:
                if monitor is not None:
                    monitor.close()
        lifecycle["process_cleanup_confirmed"] = True
