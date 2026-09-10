# SPDX-License-Identifier: Apache-2.0
"""R0 volume lineage provisioning; guest owns files, versions and receipts."""
import os
import struct
import uuid
import zlib

SECTORS = 174

def provision(fd):
    envelope = bytearray(512)
    envelope[:8] = b"RUSTVOL1"
    envelope[8:24] = uuid.uuid4().bytes
    struct.pack_into("<I", envelope, 24, zlib.crc32(envelope))
    if os.pwrite(fd, envelope, 512) != 512:
        raise RuntimeError("incomplete volume lineage write; preserve disk for recovery")
    os.fsync(fd)

def upgrade(fd, path):
    before = os.pread(fd, SECTORS * 512, 0)
    if len(before) != SECTORS * 512 or before[512:1024] != bytes(512) or any(before[160*512:]):
        raise RuntimeError("upgrade requires an unprovisioned legacy volume with empty extension sectors")
    valid = False
    for sector in (8, 13):
        h = bytearray(before[sector*512:(sector+1)*512])
        if h[:12] != b"RUSTFS1\0\x01\0\0\x02":
            continue
        checksum = struct.unpack_from("<I", h, 28)[0]
        h[28:32] = bytes(4)
        metadata = before[(sector+1)*512:(sector+5)*512]
        valid |= zlib.crc32(h) == checksum and zlib.crc32(metadata) == struct.unpack_from("<I", h, 24)[0]
    if not valid:
        raise RuntimeError("no valid legacy metadata; refusing recovery upgrade")
    backup = path.with_name(path.name + ".pre-recovery.bin")
    with backup.open("xb") as output:
        output.write(before)
        output.flush()
        os.fsync(output.fileno())
    provision(fd)
