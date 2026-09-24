# SPDX-License-Identifier: Apache-2.0
"""Independent reader for a v7 workspace volume image (#51).

Written from `docs/WORKSPACE-FORMAT7.md`, which is the contract; it does not
import, call or embed `rustic-fs`. Checksums use Python's own `zlib.crc32`
(CRC-32/IEEE) and digests use `hashlib`, so agreement with the Rust mount is
evidence rather than a restatement. The reader selects a header copy by the
format's rule, verifies every checksum the selected header names, each record
CRC, the namespace, the retained records and their bindings, exact allocation
ownership and every live and retained payload CRC. It raises `Corrupt` (an
`AssertionError`) naming the first rule it cannot confirm.
"""
import hashlib
import struct
import zlib

SECTOR = 512
MAGIC = b"RUSTFS3\0"
VERSION = 7
LAYOUT = 1
FEATURES = 0b1111  # extent payload, immutable snapshots, scoped records, 512 KiB files
HEADER_SECTOR = 8
GENERATIONS = 2
NODE_BYTES = 128
NODES = 256
NODES_SECTORS = NODES * NODE_BYTES // SECTOR
MAP_BYTES = 16384
MAP_SECTORS = MAP_BYTES // SECTOR
RECORD_BYTES = 192
RETAINED = 8
RECEIPT_BYTES = 4 * SECTOR
RECEIPTS_SECTORS = RECEIPT_BYTES // SECTOR
GENERATION_SECTORS = NODES_SECTORS + MAP_SECTORS + RECEIPTS_SECTORS
FIRST_GENERATION_SECTOR = HEADER_SECTOR + GENERATIONS
PAYLOAD_SECTOR = FIRST_GENERATION_SECTOR + GENERATIONS * GENERATION_SECTORS
DATA_SECTORS = 131072
VOLUME_SECTORS = PAYLOAD_SECTOR + DATA_SECTORS
EXTENTS = 8
MAX_FILE = 512 * 1024
NAME_BYTES = 32
NAME_MAX = 31
NEXT_MIN = 5
NEXT_EXHAUSTED = 0xFFFF_FFFF
ROOTS = ("system", "data", "config", "workspaces")
KINDS = {1: "file", 2: "directory"}
STATES = {0: "direct_committed", 1: "admitted", 2: "cancelled", 3: "admitted_committed"}
CAUSES = {0: "unknown", 1: "requested", 2: "version_conflict", 3: "authority_lost"}
COMMITTED_STATES = ("direct_committed", "admitted_committed")
NAME_BYTES_ALLOWED = frozenset(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-")


class Corrupt(AssertionError):
    """The image breaks a rule of the v7 format contract."""


def _u32(raw, at):
    return struct.unpack_from("<I", raw, at)[0]


def _u64(raw, at):
    return struct.unpack_from("<Q", raw, at)[0]


def crc32(data):
    return zlib.crc32(data) & 0xFFFF_FFFF


def header_sector(generation):
    return HEADER_SECTOR + generation


def nodes_sector(generation):
    return FIRST_GENERATION_SECTOR + generation * GENERATION_SECTORS


def map_sector(generation):
    return nodes_sector(generation) + NODES_SECTORS


def receipts_sector(generation):
    return map_sector(generation) + MAP_SECTORS


def _sectors(data, first, count):
    return data[first * SECTOR:(first + count) * SECTOR]


def decode_header(raw, slot):
    """Decode the copy stored in physical header slot `slot`, or raise `Corrupt`."""
    if len(raw) != SECTOR:
        raise Corrupt("header copy is not one sector")
    if raw[:8] != MAGIC or raw[8] != VERSION or raw[9] != LAYOUT:
        raise Corrupt("header copy is not a v7 layout-1 header")
    body = bytearray(raw)
    body[508:512] = bytes(4)
    if crc32(bytes(body)) != _u32(raw, 508):
        raise Corrupt("header copy checksum does not match")
    if raw[11] != 0 or raw[89:508] != bytes(508 - 89):
        raise Corrupt("header copy reserved bytes are not zero")
    geometry = (_u32(raw, 60), _u32(raw, 64), _u32(raw, 68), _u32(raw, 72), _u32(raw, 76), _u32(raw, 84), raw[88])
    if geometry != (NODES, NODE_BYTES, MAP_BYTES, DATA_SECTORS, RETAINED, RECORD_BYTES, EXTENTS):
        raise Corrupt("header copy geometry is not the frozen v7 geometry")
    if _u32(raw, 80) != FEATURES:
        raise Corrupt("header copy feature mask is not exactly 15")
    header = {
        "slot": slot,
        "generation": raw[10],
        "lineage": bytes(raw[12:28]),
        "epoch": _u64(raw, 28),
        "sequence": _u64(raw, 36),
        "next": _u32(raw, 44),
        "nodes_checksum": _u32(raw, 48),
        "map_checksum": _u32(raw, 52),
        "receipts_checksum": _u32(raw, 56),
    }
    if header["generation"] >= GENERATIONS:
        raise Corrupt("header copy names a generation that does not exist")
    if header["generation"] != slot:
        raise Corrupt("header copy generation does not match its physical slot")
    if header["lineage"] == bytes(16):
        raise Corrupt("header copy has no lineage")
    if header["sequence"] < 1 or header["epoch"] < 1 or header["epoch"] > header["sequence"]:
        raise Corrupt("header copy epoch/sequence invariants fail")
    if header["next"] < NEXT_MIN:
        raise Corrupt("header copy identity watermark is below the first identity")
    return header


def _genesis(header, other):
    """Generation 0 as provisioned, with the other copy never written."""
    return (header["generation"] == 0 and header["sequence"] == 1 and header["epoch"] == 1
            and header["next"] == NEXT_MIN and other == bytes(SECTOR))


def select_header(data):
    """Apply the mount selection rule to both copies.

    Returns `(header, recovered, rejected)` where `rejected` maps a refused slot to
    its reason. `recovered` is true when an invalid copy was ignored, except for a
    freshly provisioned generation 0 whose other copy is still all zero.
    """
    copies = [bytes(_sectors(data, header_sector(slot), 1)) for slot in range(GENERATIONS)]
    valid, rejected = {}, {}
    for slot, raw in enumerate(copies):
        try:
            valid[slot] = decode_header(raw, slot)
        except Corrupt as error:
            rejected[slot] = str(error)
    if not valid:
        raise Corrupt("no valid header copy: " + "; ".join(f"slot {slot}: {why}" for slot, why in rejected.items()))
    if len(valid) == 1:
        (slot, header), = valid.items()
        return header, not _genesis(header, copies[1 - slot]), rejected
    older, newer = sorted(valid.values(), key=lambda header: header["sequence"])
    if older["lineage"] != newer["lineage"]:
        raise Corrupt("valid header copies carry different lineages")
    if newer["sequence"] - older["sequence"] != 1:
        raise Corrupt("valid header copies do not have adjacent sequences")
    if newer["epoch"] < older["epoch"] or newer["next"] < older["next"]:
        raise Corrupt("newer header copy regresses the epoch or identity watermark")
    return newer, False, rejected


def _runs(raw, at, used, length, what):
    runs = [struct.unpack_from("<II", raw, at + index * 8) for index in range(EXTENTS)]
    if used > EXTENTS or length > MAX_FILE:
        raise Corrupt(f"{what} declares more runs or bytes than the format allows")
    for start, count in runs[:used]:
        if count == 0 or start + count > DATA_SECTORS:
            raise Corrupt(f"{what} has an empty or out-of-bounds run")
    for index, (start, count) in enumerate(runs[:used]):
        for other_start, other_count in runs[index + 1:used]:
            if start < other_start + other_count and other_start < start + count:
                raise Corrupt(f"{what} has overlapping runs")
    if any(start or count for start, count in runs[used:]):
        raise Corrupt(f"{what} stores runs it does not declare")
    if sum(count for _, count in runs[:used]) != (length + SECTOR - 1) // SECTOR:
        raise Corrupt(f"{what} runs do not total exactly ceil(length / 512) sectors")
    return runs[:used]


def valid_name(name):
    return (0 < len(name) <= NAME_MAX and name not in (b".", b"..")
            and all(byte in NAME_BYTES_ALLOWED for byte in name))


def decode_node(raw):
    """Decode one 128-byte node slot: `None` for the all-zero empty slot."""
    if len(raw) != NODE_BYTES:
        raise Corrupt("node slot is not 128 bytes")
    if raw == bytes(NODE_BYTES):
        return None
    if crc32(raw[:124]) != _u32(raw, 124):
        raise Corrupt("node record checksum does not match")
    kind = KINDS.get(raw[20])
    if kind is None:
        raise Corrupt("node record kind is neither file nor directory")
    node = {"id": _u32(raw, 0), "parent": _u32(raw, 4), "version": _u64(raw, 8), "length": _u32(raw, 16),
            "kind": kind, "space": raw[21], "crc": _u32(raw, 120)}
    used, name_length = raw[22], raw[23]
    if node["id"] == 0 or node["version"] == 0 or not 1 <= node["space"] <= 4:
        raise Corrupt("node record identity, version or space is not live")
    if name_length > NAME_MAX:
        raise Corrupt("node record name is longer than 31 bytes")
    name = bytes(raw[88:88 + name_length])
    if not valid_name(name) or raw[88 + name_length:120] != bytes(NAME_BYTES - name_length):
        raise Corrupt("node record name breaks the namespace rules or is not zero padded")
    if kind == "directory" and (node["length"] or node["crc"] or used):
        raise Corrupt("directory node owns payload")
    node["runs"] = _runs(raw, 24, used, node["length"], "node record")
    node["name"] = name.decode("ascii")
    return node


def decode_record(raw):
    """Decode one 192-byte retained slot: `None` for the all-zero empty slot."""
    if len(raw) != RECORD_BYTES:
        raise Corrupt("retained slot is not 192 bytes")
    if raw == bytes(RECORD_BYTES):
        return None
    if crc32(raw[:188]) != _u32(raw, 188):
        raise Corrupt("retained record checksum does not match")
    if raw[83:88] != bytes(5) or raw[152:188] != bytes(36):
        raise Corrupt("retained record reserved bytes are not zero")
    state = STATES.get(raw[80])
    if state is None:
        raise Corrupt("retained record state is unknown")
    if state == "cancelled":
        cause = CAUSES.get(raw[81])
        if cause is None:
            raise Corrupt("cancelled record prevention cause is unknown")
    elif raw[81] != 0:
        raise Corrupt("prevention cause present outside a cancelled record")
    else:
        cause = None
    record = {"subject": _u64(raw, 0), "workspace": _u32(raw, 8), "object": _u32(raw, 12),
              "instance": _u64(raw, 16), "epoch": _u64(raw, 24), "key": _u64(raw, 32),
              "previous": _u64(raw, 40), "committed": _u64(raw, 48), "admission": _u64(raw, 56),
              "terminal": _u64(raw, 64), "length": _u32(raw, 72), "crc": _u32(raw, 76),
              "state": state, "cause": cause}
    if not all(record[field] for field in ("subject", "workspace", "instance", "epoch", "key", "previous")):
        raise Corrupt("retained record scope, retry or previous field is zero")
    if record["object"] < NEXT_MIN or record["object"] == record["workspace"]:
        raise Corrupt("retained record object is a root or its own workspace")
    previous, committed, admission, terminal = (record[field] for field in
                                                ("previous", "committed", "admission", "terminal"))
    if state == "direct_committed":
        ok = admission == 0 and terminal == committed and committed > previous
        created = committed
    elif state == "admitted":
        ok = committed == 0 and terminal == 0 and admission > previous
        created = admission
    elif state == "cancelled":
        ok = committed == 0 and terminal > admission > previous
        created = admission
    else:
        ok = terminal == committed and committed > admission > previous
        created = admission
    if not ok:
        raise Corrupt(f"retained {state} record sequences break the state arithmetic")
    if record["instance"] > created or record["epoch"] > created:
        raise Corrupt("retained record instance or retry epoch postdates its creation")
    record["runs"] = _runs(raw, 88, raw[82], record["length"], "retained record")
    return record


def _generation(data, header):
    """Verify the aggregates named by the selected header and decode its regions."""
    generation = header["generation"]
    table = bytes(_sectors(data, nodes_sector(generation), NODES_SECTORS))
    if len(table) != NODES_SECTORS * SECTOR or crc32(table) != header["nodes_checksum"]:
        raise Corrupt("node table does not match the header aggregate")
    nodes = [decode_node(table[index * NODE_BYTES:(index + 1) * NODE_BYTES]) for index in range(NODES)]
    words = bytes(_sectors(data, map_sector(generation), MAP_SECTORS))
    if len(words) != MAP_BYTES or crc32(words) != header["map_checksum"]:
        raise Corrupt("allocation map does not match the header aggregate")
    allocated = set()
    for index, (word,) in enumerate(struct.iter_unpack("<Q", words)):
        while word:
            low = word & -word
            allocated.add(index * 64 + low.bit_length() - 1)
            word ^= low
    block = bytes(_sectors(data, receipts_sector(generation), RECEIPTS_SECTORS))
    if len(block) != RECEIPT_BYTES or crc32(block) != header["receipts_checksum"]:
        raise Corrupt("retained record block does not match the header aggregate")
    if block[RETAINED * RECORD_BYTES:] != bytes(RECEIPT_BYTES - RETAINED * RECORD_BYTES):
        raise Corrupt("retained record block padding is not zero")
    records = [decode_record(block[index * RECORD_BYTES:(index + 1) * RECORD_BYTES]) for index in range(RETAINED)]
    return nodes, allocated, records


def _namespace(header, nodes):
    live = {}
    siblings = set()
    for node in nodes:
        if node is None:
            continue
        if node["id"] >= header["next"] or node["version"] > header["sequence"]:
            raise Corrupt("node identity or version is beyond the header watermarks")
        if node["id"] in live:
            raise Corrupt("two nodes share one identity")
        if (node["parent"], node["name"]) in siblings:
            raise Corrupt("two siblings share one name")
        live[node["id"]] = node
        siblings.add((node["parent"], node["name"]))
        if node["parent"] == 0:
            if not 1 <= node["id"] <= 4 or node["kind"] != "directory" or node["space"] != node["id"] \
                    or node["name"] != ROOTS[node["id"] - 1]:
                raise Corrupt("a parentless node is not the matching named root")
    if any(identity not in live or live[identity]["parent"] != 0 for identity in range(1, 5)):
        raise Corrupt("the four named roots are not all present")
    paths = {}
    for node in live.values():
        chain, cursor, seen = [], node, set()
        while cursor["parent"] != 0:
            if cursor["id"] in seen:
                raise Corrupt("namespace ancestry has a cycle")
            seen.add(cursor["id"])
            parent = live.get(cursor["parent"])
            if parent is None or parent["kind"] != "directory" or parent["space"] != cursor["space"]:
                raise Corrupt("a node's parent is missing, not a directory or in another space")
            chain.append(cursor["name"])
            cursor = parent
        chain.append(cursor["name"])
        paths[node["id"]] = "/" + "/".join(reversed(chain))
    return live, paths


def _observed(record):
    """Sequence at which the record observed its previous version."""
    return record["committed"] if record["state"] == "direct_committed" else record["admission"]


def _matches_content(record, node):
    return (node["kind"] == "file" and node["length"] == record["length"] and node["crc"] == record["crc"]
            and node["runs"] == record["runs"])


def _object_history(left, right):
    """Same-object temporal rules between two retained records; returns a reason or None."""
    if _observed(left) == _observed(right):
        return "two records on one object observe it at the same sequence"
    first, second = sorted((left, right), key=_observed)
    if first["previous"] > second["previous"]:
        return "retained version observations decrease with event sequence"
    for commit, other in ((left, right), (right, left)):
        if commit["state"] == "admitted_committed" and commit["admission"] < _observed(other) < commit["committed"] \
                and other["previous"] != commit["previous"]:
            return "an observation inside an admitted commit's window saw another version"
        if commit["state"] in COMMITTED_STATES and _observed(other) > commit["committed"] \
                and other["previous"] < commit["committed"]:
            return "an observation after a commit saw an older version"
    if left["state"] in COMMITTED_STATES and right["state"] in COMMITTED_STATES:
        early, late = sorted((left, right), key=lambda record: record["committed"])
        if late["previous"] < early["committed"]:
            return "committed records do not form a monotonic history"
    return None


def _records(header, live, records):
    decoded = []
    for slot, record in enumerate(records):
        if record is None:
            continue
        if record["workspace"] >= header["next"] or record["object"] >= header["next"]:
            raise Corrupt("retained record identities are beyond the watermark")
        if max(record[field] for field in ("epoch", "previous", "committed", "admission", "terminal")) \
                > header["sequence"]:
            raise Corrupt("retained record sequences are beyond the header sequence")
        if record["epoch"] != header["epoch"]:
            raise Corrupt("retained record belongs to another retry epoch")
        node = live.get(record["object"])
        if node is not None:
            if node["kind"] != "file" or node["version"] < record["previous"]:
                raise Corrupt("retained record target is not a file at or after its previous version")
            if record["state"] in ("admitted", "cancelled") and record["previous"] < node["version"] <= record["admission"]:
                raise Corrupt("live target advanced before the record's admission")
            if record["state"] in COMMITTED_STATES:
                if node["version"] < record["committed"]:
                    raise Corrupt("live target is older than the record's commit")
                if node["version"] == record["committed"] and not _matches_content(record, node):
                    raise Corrupt("live target at the committed version differs from the snapshot")
        for prior in decoded:
            if (prior["subject"], prior["workspace"], prior["epoch"], prior["key"]) == \
                    (record["subject"], record["workspace"], record["epoch"], record["key"]):
                raise Corrupt("two retained records share one retry identity")
            events = {prior[field] for field in ("admission", "terminal", "committed")} - {0}
            if events & ({record[field] for field in ("admission", "terminal", "committed")} - {0}):
                raise Corrupt("two retained records share an event sequence")
            if prior["object"] == record["object"]:
                reason = _object_history(prior, record)
                if reason:
                    raise Corrupt(reason)
        record = dict(record, slot=slot)
        record["aliases_live"] = (record["state"] in COMMITTED_STATES and node is not None
                                  and node["version"] == record["committed"] and _matches_content(record, node))
        decoded.append(record)
    return decoded


def _ownership(live, records, allocated):
    claimed = set()

    def claim(runs, what):
        for start, count in runs:
            for sector in range(start, start + count):
                if sector in claimed:
                    raise Corrupt(f"{what} claims a payload sector that is already owned")
                claimed.add(sector)

    for node in live.values():
        if node["kind"] == "file":
            claim(node["runs"], f"file {node['id']}")
    aliased = set()
    for record in records:
        if record["length"] == 0 and record["crc"] != 0:
            raise Corrupt("an empty retained snapshot has a nonzero payload CRC")
        if record["aliases_live"]:
            if record["object"] in aliased:
                raise Corrupt("two retained records alias one live file version")
            aliased.add(record["object"])
        else:
            claim(record["runs"], f"retained slot {record['slot']}")
    if allocated - claimed:
        raise Corrupt("the allocation map marks sectors no file or snapshot owns")
    if claimed - allocated:
        raise Corrupt("a file or snapshot owns sectors the allocation map marks free")
    return claimed


def _payload(data, runs, length, expected, what):
    out = bytearray()
    for start, count in runs:
        first = PAYLOAD_SECTOR + start
        if (first + count) * SECTOR > len(data):
            raise Corrupt(f"image is truncated before the {what} payload")
        out += data[first * SECTOR:(first + count) * SECTOR]
    out = bytes(out[:length])
    if len(out) != length or crc32(out) != expected:
        raise Corrupt(f"{what} payload does not match its CRC")
    return out


def snapshot(data):
    """Read and verify a whole v7 image; return the selected generation's view."""
    data = memoryview(data)
    if len(data) != VOLUME_SECTORS * SECTOR:
        raise Corrupt(f"image is not exactly the {VOLUME_SECTORS}-sector v7 volume")
    header, recovered, rejected = select_header(data)
    nodes, allocated, records = _generation(data, header)
    live, paths = _namespace(header, nodes)
    records = _records(header, live, records)
    claimed = _ownership(live, records, allocated)
    files = []
    contents = {}
    for identity in sorted(live):
        node = live[identity]
        if node["kind"] != "file":
            continue
        payload = _payload(data, node["runs"], node["length"], node["crc"], f"file {identity}")
        contents[paths[identity]] = payload
        files.append({"path": paths[identity], "id": identity, "version": node["version"],
                      "size": node["length"], "sha256": hashlib.sha256(payload).hexdigest()})
    for record in records:
        payload = _payload(data, record["runs"], record["length"], record["crc"], f"retained slot {record['slot']}")
        record["sha256"] = hashlib.sha256(payload).hexdigest()
        target = live.get(record["object"])
        record["target"] = None if target is None else {"path": paths[record["object"]], "version": target["version"]}
    return {
        "format": VERSION,
        "lineage": header["lineage"].hex(),
        "generation": header["generation"],
        "sequence": header["sequence"],
        "epoch": header["epoch"],
        "next": header["next"],
        "recovered": recovered,
        "rejected_headers": {str(slot): reason for slot, reason in rejected.items()},
        "nodes": {identity: dict(node, path=paths[identity]) for identity, node in live.items()},
        "files": files,
        "contents": contents,
        "records": records,
        "used_sectors": len(claimed),
        "free_sectors": DATA_SECTORS - len(claimed),
    }
