# SPDX-License-Identifier: Apache-2.0
"""Temporary RUSTFS1 fixture disk for legacy acceptance harnesses only.

`terminal-test` and `recovery-test` select the private legacy file-service
branch, which mounts RUSTFS1 and initializes only reserved sectors that are all
zero apart from a valid R0 `RUSTVOL1` lineage envelope. Without that envelope
the fresh volume has no recovery state and retry keys, receipts and epochs
answer `Unsupported`, so a fresh fixture still needs host lineage provisioning.

V7 launchers use `machine.disk`/`v7_disk`; never route them here. Delete this
module with the legacy file service when RUSTFS1 is retired.
"""
import contextlib
import fcntl
import os
from pathlib import Path
import stat

from . import provision
from .machine import SIZE


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
            raise RuntimeError("legacy fixture lock must be a dedicated regular file")
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fd = os.open(path, flags, 0o600)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise RuntimeError("legacy fixture data must be a dedicated regular file")
        if initialize:
            os.ftruncate(fd, SIZE)
            provision.provision(fd)
        elif info.st_size != SIZE:
            raise RuntimeError("unexpected legacy fixture disk size; refusing to modify it")
        if upgrade_recovery:
            provision.upgrade(fd, path)
        yield path.resolve()
    finally:
        if fd is not None:
            os.close(fd)
        os.close(lock_fd)
