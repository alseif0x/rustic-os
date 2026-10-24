# SPDX-License-Identifier: Apache-2.0
"""Independent V7 genesis image and provision7 test double."""
from pathlib import Path
import struct
import zlib

from terminal_support import v7_disk


def genesis(path, lineage=bytes(range(1, 17))):
    table = bytearray(32768)
    for index, name in enumerate((b"system", b"data", b"config", b"workspaces")):
        node = bytearray(128)
        struct.pack_into("<IIQI", node, 0, index + 1, 0, 1, 0)
        node[20:24] = bytes((2, index + 1, 0, len(name)))
        node[88:88 + len(name)] = name
        struct.pack_into("<I", node, 124, zlib.crc32(node[:124]))
        table[index * 128:(index + 1) * 128] = node
    header = bytearray(512)
    header[:8] = b"RUSTFS3\0"
    header[8:11] = bytes((7, 1, 0))
    header[12:28] = lineage
    struct.pack_into("<QQI", header, 28, 1, 1, 5)
    struct.pack_into("<III", header, 48, zlib.crc32(table),
                     zlib.crc32(bytes(16384)), zlib.crc32(bytes(2048)))
    struct.pack_into("<IIIIIIIB", header, 60, 256, 128, 16384, 131072, 8, 15, 192, 8)
    struct.pack_into("<I", header, 508, zlib.crc32(header))
    with Path(path).open("xb") as out:
        out.truncate(v7_disk.PREFIX_BYTES)
        out.seek(8 * 512)
        out.write(header)
        out.seek(10 * 512)
        out.write(table)


def provision(command, **kwargs):
    assert command[1] == "provision7"
    assert len(command[3]) == 32 and int(command[3], 16) != 0
    genesis(command[2], bytes.fromhex(command[3]))
