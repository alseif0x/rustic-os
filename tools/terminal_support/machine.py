# SPDX-License-Identifier: Apache-2.0
"""Own only a dedicated regular data image and a single reference QEMU process."""
import contextlib
import fcntl
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import environment

SIZE = 4 * 1024 ** 3

@contextlib.contextmanager
def disk(path, initialize=False, upgrade_recovery=False):
    if initialize and upgrade_recovery:
        raise RuntimeError("initialize and upgrade are mutually exclusive")
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_RDWR | os.O_NOFOLLOW | (os.O_CREAT | os.O_EXCL if initialize else 0)
    # A separate lock coordinates launchers. DrvFS translates flock on the data
    # file into locks that conflict with QEMU's own byte-range drive locking.
    # Keep QEMU's drive locks enabled; do not weaken them to mask that conflict.
    lock_path = path.with_name(path.name + ".lock")
    lock_fd = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    fd = None
    try:
        lock_info = os.fstat(lock_fd)
        if not stat.S_ISREG(lock_info.st_mode) or lock_info.st_nlink != 1:
            raise RuntimeError("terminal lock must be a dedicated regular file")
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fd = os.open(path, flags, 0o600)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise RuntimeError("terminal data must be a dedicated regular file")
        if initialize:
            os.ftruncate(fd, SIZE)
            from .provision import provision
            provision(fd)
        elif info.st_size != SIZE:
            raise RuntimeError("unexpected terminal disk size; refusing to modify it")
        if upgrade_recovery:
            from .provision import upgrade
            upgrade(fd, path)
        yield path.resolve()
    finally:
        if fd is not None:
            os.close(fd)
        os.close(lock_fd)

@contextlib.contextmanager
def machine(image, data, serial, log, fault=None):
    image = Path(image).resolve()
    metadata = json.loads((image.parent / "image.json").read_text())
    if environment.digest(image) != metadata["image_sha256"]:
        raise RuntimeError("boot image changed after construction")
    with tempfile.TemporaryDirectory(prefix="rustic-terminal-") as temporary:
        variables = Path(temporary) / "OVMF_VARS.fd"
        shutil.copyfile("/usr/share/OVMF/OVMF_VARS_4M.fd", variables)
        config = environment.CONFIG
        drive = str(data)
        if fault is not None:
            from .recovery_faults import configuration
            rules = Path(temporary) / "blkdebug.conf"
            rules.write_text(configuration(fault))
            drive = f"blkdebug:{rules}:{data}"
        command = [
            "qemu-system-x86_64", "-machine", config["machine"], "-accel", config["accelerator"],
            "-cpu", config["cpu"], "-smp", "1", "-m", "256M",
            "-drive", "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd",
            "-drive", f"if=pflash,format=raw,file={variables}",
            "-drive", f"if=virtio,format=raw,readonly=on,file={image}",
            "-drive", f"if=none,id=rusticdata,format=raw,cache=writeback,file={drive}",
            "-device", "virtio-blk-pci,drive=rusticdata,addr=0x6,disable-modern=on,disable-legacy=off,queue-size=8,num-queues=1,vectors=0,rerror=report,werror=report",
            "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-display", "none", "-serial", serial, "-monitor", "none", "-nic", "none", "-no-reboot",
        ]
        if serial == "stdio":
            position=command.index("-serial")
            command[position:position+2]=["-chardev","stdio,id=rusticconsole,signal=off,mux=on","-serial","chardev:rusticconsole"]
        with Path(log).open("wb") as errors:
            process = subprocess.Popen(command, stderr=errors)
            try:
                yield process
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
