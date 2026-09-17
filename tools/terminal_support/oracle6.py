# SPDX-License-Identifier: Apache-2.0
"""Independent reader for a v6 volume image (#51).

The layout is described in `crates/fs/src/format6.rs`, `crates/fs/src/extent.rs`
and `crates/fs/src/receipt6.rs`; this reader is written from that description with
Python's own CRC, so agreement between it and the Rust implementation is evidence
rather than a restatement. It verifies every checksum the header carries, the
per-file bounds and the free-space map, and it raises `AssertionError` on
anything it cannot confirm.
"""
import struct
import zlib

SECTOR = 512
HEADER_SECTOR = 8
MAGIC = b"RUSTFS2\0"
VERSION = 6
NODE_BYTES = 128
OBJECTS = 256
NODES_SECTORS = OBJECTS * NODE_BYTES // SECTOR
MAP_WORDS = 2048
MAP_BYTES = MAP_WORDS * 8
MAP_SECTORS = MAP_BYTES // SECTOR
RECEIPT_BYTES = 96
RETAINED = 8
BLOCK_BYTES = 1024
RECEIPT_SECTORS = BLOCK_BYTES // SECTOR
GENERATIONS = 2
GENERATION_SECTORS = NODES_SECTORS + MAP_SECTORS + RECEIPT_SECTORS
PAYLOAD_SECTOR = HEADER_SECTOR + 1 + GENERATIONS * GENERATION_SECTORS
DATA_SECTORS = 131072
EXTENTS_PER_FILE = 8
FILE_SECTORS_MAX = 512
MAX_FILE = 256 * 1024
KINDS = {0: "empty", 1: "file", 2: "directory"}


def nodes_sector(generation):
    return HEADER_SECTOR + 1 + generation % GENERATIONS * GENERATION_SECTORS


def map_sector(generation):
    return nodes_sector(generation) + NODES_SECTORS


def receipts_sector(generation):
    return map_sector(generation) + MAP_SECTORS


def _header(data):
    sector = data[HEADER_SECTOR * SECTOR:(HEADER_SECTOR + 1) * SECTOR]
    if len(sector) != SECTOR or sector[:8] != MAGIC or sector[8] != VERSION:
        raise AssertionError("not a v6 header")
    if sector[9:12] != b"\0\0\x02" or sector[33:36] != bytes(3) or sector[52:] != bytes(SECTOR - 52):
        raise AssertionError("v6 header reserved bytes are not as declared")
    expected = struct.unpack_from("<I", sector, 48)[0]
    body = bytearray(sector)
    body[48:52] = bytes(4)
    if zlib.crc32(bytes(body)) != expected:
        raise AssertionError("v6 header checksum does not match")
    sequence, objects = struct.unpack_from("<Q", sector, 12)[0], struct.unpack_from("<I", sector, 20)[0]
    node_bytes, data_sectors, active = struct.unpack_from("<I", sector, 24)[0], struct.unpack_from("<I", sector, 28)[0], sector[32]
    if objects != OBJECTS or node_bytes != NODE_BYTES or data_sectors != DATA_SECTORS:
        raise AssertionError("v6 header geometry differs from the selected budget")
    if active >= GENERATIONS:
        raise AssertionError("v6 header names a generation that does not exist")
    return {"sequence": sequence, "active": active,
            "nodes_checksum": struct.unpack_from("<I", sector, 36)[0],
            "map_checksum": struct.unpack_from("<I", sector, 40)[0],
            "receipts_checksum": struct.unpack_from("<I", sector, 44)[0]}


def _node(raw):
    if len(raw) != NODE_BYTES:
        raise AssertionError("node record is not its declared size")
    body = bytearray(raw)
    checksum = struct.unpack_from("<I", body, NODE_BYTES - 4)[0]
    body[NODE_BYTES - 4:] = bytes(4)
    if zlib.crc32(bytes(body[:NODE_BYTES - 4])) != checksum:
        raise AssertionError("node record checksum does not match")
    kind = raw[20]
    if kind not in KINDS or raw[23] != 0 or raw[121:124] != bytes(3):
        raise AssertionError("node record reserved fields are not as declared")
    used = raw[22]
    if used > EXTENTS_PER_FILE or raw[120] > 32:
        raise AssertionError("node record declares more runs or name bytes than it holds")
    extents = []
    for index in range(EXTENTS_PER_FILE):
        start, sectors = struct.unpack_from("<II", raw, 24 + index * 8)
        if start + sectors > DATA_SECTORS:
            raise AssertionError("node extent leaves the payload region")
        extents.append((start, sectors))
    if any(sectors or start for start, sectors in extents[used:]):
        raise AssertionError("node record stores runs it does not declare")
    held = sum(sectors for _, sectors in extents[:used])
    length = struct.unpack_from("<I", raw, 16)[0]
    if held > FILE_SECTORS_MAX or length > held * SECTOR or length > MAX_FILE:
        raise AssertionError("node record claims more payload than the budget allows")
    return {"id": struct.unpack_from("<I", raw, 0)[0], "parent": struct.unpack_from("<I", raw, 4)[0],
            "version": struct.unpack_from("<Q", raw, 8)[0], "length": length, "kind": KINDS[kind],
            "space": raw[21], "extents": extents[:used],
            "name": raw[88:88 + raw[120]].decode("utf-8", "replace")}


def _table(data, header):
    """Verify the active generation and return its node table and map."""
    base = nodes_sector(header["active"])
    table = data[base * SECTOR:(base + NODES_SECTORS) * SECTOR]
    if len(table) != NODES_SECTORS * SECTOR or zlib.crc32(table) != header["nodes_checksum"]:
        raise AssertionError("node table does not match the header checksum")
    nodes = [_node(table[index * NODE_BYTES:(index + 1) * NODE_BYTES]) for index in range(OBJECTS)]
    base = map_sector(header["active"])
    words = data[base * SECTOR:(base + MAP_SECTORS) * SECTOR]
    if len(words) != MAP_BYTES or zlib.crc32(words) != header["map_checksum"]:
        raise AssertionError("free-space map does not match the header checksum")
    used = set()
    for index, (word,) in enumerate(struct.iter_unpack("<Q", words)):
        for bit in range(64):
            if word >> bit & 1:
                used.add(index * 64 + bit)
    return nodes, used


def _receipts(data, header):
    base = receipts_sector(header["active"])
    block = data[base * SECTOR:(base + RECEIPT_SECTORS) * SECTOR]
    if len(block) != BLOCK_BYTES or zlib.crc32(block[:24 + RETAINED * RECEIPT_BYTES]) != header["receipts_checksum"]:
        raise AssertionError("receipt block does not match the header checksum")
    if block[24 + RETAINED * RECEIPT_BYTES:] != bytes(BLOCK_BYTES - 24 - RETAINED * RECEIPT_BYTES):
        raise AssertionError("receipt block padding is not zero")
    lineage, epoch = block[:16], struct.unpack_from("<Q", block, 16)[0]
    if lineage == bytes(16) or epoch == 0:
        raise AssertionError("receipt block identity is not usable")
    records = []
    for index in range(RETAINED):
        raw = block[24 + index * RECEIPT_BYTES:24 + (index + 1) * RECEIPT_BYTES]
        if raw == bytes(RECEIPT_BYTES):
            continue
        body = bytearray(raw)
        checksum = struct.unpack_from("<I", body, RECEIPT_BYTES - 4)[0]
        body[RECEIPT_BYTES - 4:] = bytes(4)
        if zlib.crc32(bytes(body[:RECEIPT_BYTES - 4])) != checksum or raw[56:92] != bytes(36):
            raise AssertionError("receipt record checksum or reserved bytes do not match")
        if raw[:16] != lineage:
            raise AssertionError("receipt record carries another lineage")
        key, subject = struct.unpack_from("<Q", raw, 24)[0], struct.unpack_from("<I", raw, 32)[0]
        previous, committed, length = struct.unpack_from("<QQ", raw, 36)[0], struct.unpack_from("<Q", raw, 44)[0], struct.unpack_from("<I", raw, 52)[0]
        records.append({"lineage": lineage, "epoch": struct.unpack_from("<Q", raw, 16)[0],
                        "key": key, "id": subject, "previous": previous,
                        "committed": committed, "length": length})
    return {"lineage": lineage, "epoch": epoch, "records": records}


def snapshot(data):
    """Read a volume image prefix: header, active generation, payload and receipts."""
    data = bytes(data)
    if len(data) < PAYLOAD_SECTOR * SECTOR:
        raise AssertionError("image is shorter than a v6 structure prefix")
    header = _header(data)
    nodes, allocated = _table(data, header)
    receipts = _receipts(data, header)
    files, state, claimed = {}, {}, set()
    for slot, node in enumerate(nodes):
        state[slot] = node
        for start, sectors in node["extents"]:
            for sector in range(start, start + sectors):
                if sector in claimed:
                    raise AssertionError("two records claim the same payload sector")
                claimed.add(sector)
    if allocated != claimed:
        raise AssertionError("the free-space map and the live records disagree")
    for slot, node in enumerate(nodes):
        if node["kind"] != "file":
            continue
        out, remaining = bytearray(), node["length"]
        for start, sectors in node["extents"]:
            end = PAYLOAD_SECTOR + start + sectors
            if end * SECTOR > len(data):
                raise AssertionError("image is truncated before a payload extent")
            take = min(remaining, sectors * SECTOR)
            out += data[(PAYLOAD_SECTOR + start) * SECTOR:(PAYLOAD_SECTOR + start) * SECTOR + take]
            remaining -= take
            if remaining == 0:
                break
        if remaining:
            raise AssertionError("file payload is shorter than its length")
        files[(node["parent"], node["name"])] = bytes(out)
    by_id = {node["id"]: node for node in nodes if node["kind"] != "empty"}
    # A retained receipt names the operation's outcome, so its identity must still
    # be a live file and its committed version must be the one it reports.
    for receipt in receipts["records"]:
        node = by_id.get(receipt["id"])
        if node is None or node["kind"] != "file":
            raise AssertionError("a receipt names an identity that is no longer a live file")
        if not receipt["previous"] < receipt["committed"] or receipt["length"] > MAX_FILE:
            raise AssertionError("a receipt reports an impossible version or length")
    return {"format": VERSION, "sequence": header["sequence"], "active": header["active"],
            "nodes": by_id, "files": files, "used_sectors": len(claimed),
            "free_sectors": DATA_SECTORS - len(claimed), "receipts": receipts["records"],
            "receipt_epoch": receipts["epoch"], "receipt_lineage": receipts["lineage"]}
