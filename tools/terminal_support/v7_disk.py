# SPDX-License-Identifier: Apache-2.0
"""Explicitly provision or validate the manual terminal's dedicated V7 device.

The R0 device remains 4 GiB. Its V7 prefix has the frozen format's bounded
capacity; trailing device sectors do not extend the filesystem.
"""
import contextlib
import fcntl
import os
from pathlib import Path
import stat
import subprocess
import uuid

from . import oracle7
from .machine import SIZE

PREFIX_BYTES = oracle7.VOLUME_SECTORS * oracle7.SECTOR


def dedicated(fd, kind):
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise RuntimeError(f"terminal {kind} must be a dedicated regular file")
    return info


def same_file(path, info):
    current = os.stat(path, follow_symlinks=False)
    if (current.st_dev, current.st_ino) != (info.st_dev, info.st_ino):
        raise RuntimeError("terminal disk path changed; refusing to launch")


def snapshot(fd):
    """Verify only the frozen V7 prefix, without relaxing exact-image tools."""
    try:
        return oracle7.snapshot(os.pread(fd, PREFIX_BYTES, 0))
    except oracle7.Corrupt as error:
        raise RuntimeError("terminal disk is not mountable V7; no conversion is performed") from error


@contextlib.contextmanager
def disk(path, initialize=False, *, volume_tool=None):
    path = Path(path).absolute()
    path.parent.mkdir(parents=True, exist_ok=True)
    lock_path = path.with_name(path.name + ".lock")
    lock_fd = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    fd = None
    try:
        dedicated(lock_fd, "lock")
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if initialize:
            if volume_tool is None:
                raise ValueError("fresh V7 provisioning requires the volume tool")
            # The formatter uses create_new. Never create or truncate the data
            # path before it has exclusively provisioned a fresh exact image.
            subprocess.run([str(volume_tool), "provision7", str(path), uuid.uuid4().hex],
                           check=True, stdout=subprocess.DEVNULL)
        fd = os.open(path, os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
        info = dedicated(fd, "data")
        same_file(path, info)
        required = PREFIX_BYTES if initialize else SIZE
        if info.st_size != required:
            raise RuntimeError("unexpected terminal disk size; refusing to modify it")
        snapshot(fd)
        if initialize:
            os.fchmod(fd, 0o600)
            os.ftruncate(fd, SIZE)
            os.fsync(fd)
        same_file(path, dedicated(fd, "data"))
        yield path
    finally:
        if fd is not None:
            os.close(fd)
        os.close(lock_fd)
