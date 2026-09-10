# SPDX-License-Identifier: Apache-2.0
"""Independent bounded reader for acceptance evidence, using Python's CRC implementation."""
import hashlib
import struct
import zlib

def inspect(data):
    with data.open("rb") as stream:
        selected = stream.read(160 * 512)
        stream.seek(-512, 2)
        end = stream.read(512)
    if selected[:512] != bytes(512) or end != bytes(512):
        raise AssertionError("filesystem touched reserved first/last sectors")
    banks = []
    for sector in (8, 13):
        header = bytearray(selected[sector*512:(sector+1)*512])
        if header[:12] != b"RUSTFS1\0\x01\0\0\x02":
            continue
        expected = struct.unpack_from("<I", header, 28)[0]
        header[28:32] = bytes(4)
        metadata = selected[(sector+1)*512:(sector+5)*512]
        if zlib.crc32(header) != expected or zlib.crc32(metadata) != struct.unpack_from("<I",header,24)[0]:
            continue
        sequence = struct.unpack_from("<Q",header,12)[0]
        banks.append((sequence, metadata))
    if not banks:
        raise AssertionError("no committed filesystem metadata bank")
    sequence, metadata = max(banks)
    files = {}
    for slot in range(32):
        node = metadata[slot*64:(slot+1)*64]
        if node[0] != 1:
            continue
        parent = struct.unpack_from("<I",node,4)[0]
        length = struct.unpack_from("<H",node,12)[0]
        start = (32 + slot*4 + node[2]*2) * 512
        content = selected[start:start+1024]
        if length and zlib.crc32(content[:length]) != struct.unpack_from("<I",node,24)[0]:
            raise AssertionError("file data checksum mismatch")
        files[(parent,node[32:32+node[3]].decode("ascii"))] = content[:length]
    if len(files)!=2:
        raise AssertionError("temporary files were not reclaimed")
    if files.get((4,"hello")) != b"Hello from native Rust":
        raise AssertionError("independent host read differs from committed hello content")
    if files.get((3,"owner-policy")) != b"rustic-owner-v1\nhelpers=explicit\n":
        raise AssertionError("owner policy did not persist")
    return selected, {"sequence":sequence,"file_count":len(files),"selected_sha256":hashlib.sha256(selected).hexdigest()}
