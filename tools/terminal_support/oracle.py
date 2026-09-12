# SPDX-License-Identifier: Apache-2.0
"""Independent bounded reader for acceptance evidence, using Python's CRC implementation."""
import hashlib
import struct
import zlib
from . import oracle_admission

def snapshot(data):
    with data.open("rb") as stream:
        selected = stream.read(174 * 512)
        stream.seek(-512, 2)
        end = stream.read(512)
    if selected[:512] != bytes(512) or end != bytes(512):
        raise AssertionError("filesystem touched reserved first/last sectors")
    banks = []
    for sector in (8, 13):
        header = bytearray(selected[sector*512:(sector+1)*512])
        if header[:8] != b"RUSTFS1\0" or header[8] not in (1, 2, 3, 4, 5) or header[9:12] != b"\0\0\x02":
            continue
        expected = struct.unpack_from("<I", header, 28)[0]
        header[28:32] = bytes(4)
        metadata = selected[(sector+1)*512:(sector+5)*512]
        if zlib.crc32(header) != expected or zlib.crc32(metadata) != struct.unpack_from("<I",header,24)[0]:
            continue
        sequence = struct.unpack_from("<Q",header,12)[0]
        recovery = None
        if header[8] >= 2:
            start = (160 + (sector == 13) * 7) * 512
            recovery = selected[start:start + 7*512]
            if zlib.crc32(recovery) != struct.unpack_from("<I", header, 32)[0]:
                continue
        banks.append((sequence, metadata, recovery, header[8], struct.unpack_from("<I", header, 20)[0]))
    if not banks:
        raise AssertionError("no committed filesystem metadata bank")
    sequence, metadata, recovery, version, next_id = max(banks, key=lambda b: b[0])
    files, nodes = {}, {}
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
        nodes[struct.unpack_from("<I",node,8)[0]] = {"version":struct.unpack_from("<Q",node,16)[0],"content":content[:length]}
    records = []
    if recovery is not None:
        if recovery[:8] != {2: b"RUSTREC1", 3: b"RUSTREC2", 4: b"RUSTREC3", 5: b"RUSTREC4"}[version] or any(recovery[32:512]):
            raise AssertionError("invalid recovery state")
        lineage = recovery[8:24].hex()
        envelope = bytearray(selected[512:1024])
        checksum = struct.unpack_from("<I",envelope,24)[0]
        envelope[24:28] = bytes(4)
        if envelope[:8] != b"RUSTVOL1" or envelope[8:24] != recovery[8:24] or zlib.crc32(envelope) != checksum:
            raise AssertionError("lineage envelope differs from committed recovery identity")
        epoch = struct.unpack_from("<Q", recovery, 24)[0]
        for slot in range(2):
            p = recovery[512+slot*1536:512+(slot+1)*1536]
            if not any(p):
                continue
            length = struct.unpack_from("<H",p,28)[0]
            if length > 1024 or any(p[30:32]) or any(p[52:56]) or version == 2 and any(p[48:64]) or any(p[512+length:]):
                raise AssertionError("invalid receipt padding/length")
            records.append({"subject":struct.unpack_from("<Q",p)[0],"epoch":struct.unpack_from("<Q",p,8)[0],"key":struct.unpack_from("<Q",p,16)[0],"id":struct.unpack_from("<I",p,24)[0],"previous":struct.unpack_from("<Q",p,32)[0],"committed":struct.unpack_from("<Q",p,40)[0],"content":p[512:512+length]})
            workspace = struct.unpack_from("<I", p, 48)[0]
            instance = struct.unpack_from("<Q", p, 56)[0]
            record = records[-1]
            if workspace or instance:
                if not (0 < workspace < next_id and workspace != record["id"] and instance > 0):
                    raise AssertionError("invalid historical operation namespace")
                record.update(workspace=workspace, instance=instance, sha256=hashlib.sha256(record["content"]).hexdigest())
            oracle_admission.decode(p, version, record, sequence)
            bound = record.get("admission", record["committed"])
            if not (record["subject"] > 0 and record["key"] > 0 and record["epoch"] == epoch and 4 < record["id"] < next_id and 0 < record["previous"] < bound <= sequence and instance <= bound):
                raise AssertionError("invalid retained operation identity")
            if any(oracle_admission.numbers(old) & oracle_admission.numbers(record) or (old["subject"], old.get("workspace"), old["key"]) == (record["subject"], record.get("workspace"), record["key"]) for old in records[:-1]):
                raise AssertionError("duplicate retained operation identity")
    else:
        lineage, epoch = None, None
    return selected, {"format":version,"sequence":sequence,"files":files,"nodes":nodes,"records":records,"lineage":lineage,"epoch":epoch,"selected_sha256":hashlib.sha256(selected).hexdigest()}

def inspect(data):
    selected, state = snapshot(data)
    files = state["files"]
    sequence = state["sequence"]
    if len(files)!=2:
        raise AssertionError("temporary files were not reclaimed")
    if files.get((4,"hello")) != b"Hello from native Rust":
        raise AssertionError("independent host read differs from committed hello content")
    if files.get((3,"owner-policy")) != b"rustic-owner-v1\nhelpers=explicit\n":
        raise AssertionError("owner policy did not persist")
    return selected, {"sequence":sequence,"file_count":len(files),"selected_sha256":hashlib.sha256(selected).hexdigest()}
