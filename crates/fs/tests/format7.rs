// SPDX-License-Identifier: Apache-2.0
//! Host tests for the v7 immutable-content workspace contract (#51). Codec only:
//! these tests prove the shapes, the offsets and the invariants one record can
//! prove, not that any volume mounts or serves a request.
use rustic_fs::format7::{
    FEATURES, GENERATION_SECTORS, Header7, LAYOUT, MAGIC, MAP_BYTES, MAP_SECTORS, MAX_EXTENTS,
    MAX_FILE_BYTES, NODE_BYTES, NODES, NODES_SECTORS, Node7, PAYLOAD_BYTES, PAYLOAD_SECTOR,
    RECEIPT_BLOCK_BYTES, RECEIPT_RESERVED_BYTES, RECEIPTS_SECTORS, RECORD_BYTES, RECORDS_BYTES,
    RETAINED, Record7, RecordState, SECTOR_BYTES, VERSION, VOLUME_SECTORS, aggregate,
    header_sector, map_sector, nodes_sector, receipt_slots, receipts_sector,
};
use rustic_fs::{DATA_BYTES_V6, DATA_SECTORS, Error, Extent, Header6, Kind, PreventionReason};

const LINEAGE: [u8; 16] = [0x5a; 16];

/// An independent CRC-32/IEEE, written from the polynomial so the fixed vectors
/// below do not lean on the crate's own implementation.
fn reference_crc(bytes: &[u8]) -> u32 {
    let mut value = 0xffff_ffffu32;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(value & 1));
        }
    }
    !value
}

fn runs(pairs: &[(u64, u64)]) -> [Extent; MAX_EXTENTS] {
    let mut list = [Extent::new(0, 0); MAX_EXTENTS];
    for (run, (start, sectors)) in list.iter_mut().zip(pairs) {
        *run = Extent::new(*start, *sectors);
    }
    list
}

fn seal_node(b: &mut [u8; NODE_BYTES]) {
    let checksum = reference_crc(&b[..124]);
    b[124..128].copy_from_slice(&checksum.to_le_bytes());
}

fn seal_record(b: &mut [u8; RECORD_BYTES]) {
    let checksum = reference_crc(&b[..188]);
    b[188..192].copy_from_slice(&checksum.to_le_bytes());
}

/// Change one field of an already valid encoding and re-seal it with the
/// independent CRC, so a refusal can only come from the guard that field feeds
/// and not from a stale checksum or a missing field elsewhere in the record.
fn tamper(fixture: &[u8; RECORD_BYTES], at: usize, value: &[u8]) -> [u8; RECORD_BYTES] {
    let mut b = *fixture;
    b[at..at + value.len()].copy_from_slice(value);
    seal_record(&mut b);
    b
}

fn seal_header(b: &mut [u8; SECTOR_BYTES as usize]) {
    b[508..512].fill(0);
    let checksum = reference_crc(b);
    b[508..512].copy_from_slice(&checksum.to_le_bytes());
}

/// A live file whose runs total exactly the sectors its length needs.
fn node() -> Node7 {
    let mut name = [0u8; 32];
    name[..3].copy_from_slice(b"app");
    Node7 {
        id: 7,
        parent: 5,
        version: 12,
        // Past the 16-bit length the v5 record carries.
        length: 200_000,
        kind: Kind::File,
        space: 1,
        extents_used: 3,
        extents: runs(&[(0, 64), (200, 128), (1000, 199)]),
        name_length: 3,
        name,
        payload_crc32: 0xdead_beef,
    }
}

fn direct_committed() -> Record7 {
    Record7 {
        subject: 9,
        workspace: 3,
        object: 7,
        instance: 4,
        retry_epoch: 2,
        retry_key: 11,
        previous: 3,
        committed: 4,
        admission_number: 0,
        terminal: 4,
        length: 200_000,
        payload_crc32: 0xfeed_face,
        state: RecordState::DirectCommitted,
        prevention: None,
        extents_used: 3,
        extents: runs(&[(0, 64), (200, 128), (1000, 199)]),
    }
}

fn admitted() -> Record7 {
    Record7 {
        admission_number: 4,
        committed: 0,
        terminal: 0,
        state: RecordState::Admitted,
        ..direct_committed()
    }
}

fn cancelled() -> Record7 {
    Record7 {
        admission_number: 4,
        committed: 0,
        terminal: 6,
        state: RecordState::Cancelled,
        prevention: Some(PreventionReason::Requested),
        ..direct_committed()
    }
}

fn admitted_committed() -> Record7 {
    Record7 {
        admission_number: 3,
        committed: 5,
        terminal: 5,
        previous: 2,
        instance: 3,
        state: RecordState::AdmittedCommitted,
        ..direct_committed()
    }
}

#[test]
fn the_v7_geometry_is_the_selected_budget_and_fits_the_disk() {
    assert_eq!(MAGIC, *b"RUSTFS3\0");
    assert_eq!(VERSION, 7);
    assert_eq!((NODES, NODE_BYTES, NODES_SECTORS), (256, 128, 64));
    assert_eq!((MAP_BYTES, MAP_SECTORS), (16 * 1024, 32));
    assert_eq!((RECORD_BYTES, RETAINED, RECORDS_BYTES), (192, 8, 1536));
    assert_eq!(
        (
            RECEIPT_BLOCK_BYTES,
            RECEIPT_RESERVED_BYTES,
            RECEIPTS_SECTORS
        ),
        (2048, 512, 4)
    );
    assert_eq!(GENERATION_SECTORS, 64 + 32 + 4);
    assert_eq!(MAX_FILE_BYTES, 256 * 1024);
    assert_eq!(MAX_EXTENTS, 8);
    assert_eq!(PAYLOAD_BYTES, 64 * 1024 * 1024);
    // Two header sectors keyed by generation, then two 100-sector generations.
    assert_eq!(header_sector(0), 8);
    assert_eq!(header_sector(1), 9);
    assert_eq!(nodes_sector(0), 10);
    assert_eq!(map_sector(0), 74);
    assert_eq!(receipts_sector(0), 106);
    assert_eq!(nodes_sector(1), 110);
    assert_eq!(map_sector(1), 174);
    assert_eq!(receipts_sector(1), 206);
    assert_eq!(PAYLOAD_SECTOR, 210);
    assert_eq!(VOLUME_SECTORS, PAYLOAD_SECTOR + DATA_SECTORS);
    // The generation regions tile exactly, without overlapping the headers or
    // the payload.
    assert_eq!(nodes_sector(1), nodes_sector(0) + GENERATION_SECTORS);
    assert_eq!(receipts_sector(1) + RECEIPTS_SECTORS, PAYLOAD_SECTOR);
    assert_eq!(DATA_BYTES_V6, PAYLOAD_BYTES);
    const { assert!(VOLUME_SECTORS * SECTOR_BYTES <= 4 * 1024 * 1024 * 1024) };
}

#[test]
fn the_header_offsets_and_checksum_are_fixed() {
    let header = Header7 {
        lineage: LINEAGE,
        epoch: 3,
        sequence: 9,
        next: 42,
        generation: 1,
        nodes_checksum: 0x1122_3344,
        map_checksum: 0x5566_7788,
        receipts_checksum: 0x99aa_bbcc,
    };
    let b = header.encode().unwrap();
    // An independent vector for CRC-32/IEEE justifies using the same reference
    // below for the records.
    assert_eq!(reference_crc(b"123456789"), 0xcbf4_3926);
    assert_eq!(&b[0..8], &MAGIC);
    assert_eq!(b[8], 7);
    assert_eq!(b[9], LAYOUT);
    assert_eq!(b[10], 1);
    assert_eq!(b[11], 0);
    assert_eq!(&b[12..28], &LINEAGE);
    assert_eq!(&b[28..36], &3u64.to_le_bytes());
    assert_eq!(&b[36..44], &9u64.to_le_bytes());
    assert_eq!(&b[44..48], &42u32.to_le_bytes());
    assert_eq!(&b[48..52], &0x1122_3344u32.to_le_bytes());
    assert_eq!(&b[52..56], &0x5566_7788u32.to_le_bytes());
    assert_eq!(&b[56..60], &0x99aa_bbccu32.to_le_bytes());
    assert_eq!(&b[60..64], &(NODES as u32).to_le_bytes());
    assert_eq!(&b[64..68], &(NODE_BYTES as u32).to_le_bytes());
    assert_eq!(&b[68..72], &(MAP_BYTES as u32).to_le_bytes());
    assert_eq!(&b[72..76], &(DATA_SECTORS as u32).to_le_bytes());
    assert_eq!(&b[76..80], &(RETAINED as u32).to_le_bytes());
    assert_eq!(&b[80..84], &FEATURES.to_le_bytes());
    assert_eq!(&b[84..88], &(RECORD_BYTES as u32).to_le_bytes());
    assert_eq!(b[88], MAX_EXTENTS as u8);
    assert_eq!(&b[89..92], &[0; 3]);
    assert!(b[92..508].iter().all(|byte| *byte == 0));
    let mut body = b;
    body[508..512].fill(0);
    assert_eq!(
        u32::from_le_bytes(b[508..512].try_into().unwrap()),
        reference_crc(&body)
    );
    assert_eq!(Header7::decode(&b).unwrap(), header);
    // A fresh volume starts at generation 0 with the first identity unallocated.
    let initial = Header7::initial(LINEAGE);
    assert_eq!(
        (initial.generation, initial.sequence, initial.epoch),
        (0, 1, 1)
    );
    assert_eq!(initial.next, Header7::NEXT_MIN);
    assert_eq!(
        Header7::decode(&initial.encode().unwrap()).unwrap(),
        initial
    );
}

#[test]
fn the_header_refuses_legacy_volumes_by_inspection() {
    // The frozen v6 header still has a valid checksum of its own format, and is
    // refused because the format marker and layout are not v7.
    let v6 = Header6::initial().encode();
    assert_eq!(Header7::decode(&v6), Err(Error::Corrupt));
    let mut forged = Header7::initial(LINEAGE).encode().unwrap();
    forged[..8].copy_from_slice(b"RUSTFS2\0");
    seal_header(&mut forged);
    assert_eq!(Header7::decode(&forged), Err(Error::Corrupt));
    let mut forged = Header7::initial(LINEAGE).encode().unwrap();
    forged[8] = 6;
    seal_header(&mut forged);
    assert_eq!(Header7::decode(&forged), Err(Error::Corrupt));
    let mut forged = Header7::initial(LINEAGE).encode().unwrap();
    forged[9] = LAYOUT + 1;
    seal_header(&mut forged);
    assert_eq!(Header7::decode(&forged), Err(Error::Corrupt));
}

#[test]
fn header_reserved_bytes_and_geometry_are_strict_even_with_a_valid_checksum() {
    for offset in [11, 89, 90, 91, 92, 300, 507] {
        let mut b = Header7::initial(LINEAGE).encode().unwrap();
        b[offset] = 1;
        seal_header(&mut b);
        assert_eq!(Header7::decode(&b), Err(Error::Corrupt), "offset {offset}");
    }
    // Each frozen geometry field must hold exactly.
    let mutations: [(usize, [u8; 4]); 7] = [
        (60, (NODES as u32 - 1).to_le_bytes()),
        (64, (NODE_BYTES as u32 * 2).to_le_bytes()),
        (68, (MAP_BYTES as u32 / 2).to_le_bytes()),
        (72, (DATA_SECTORS as u32 + 1).to_le_bytes()),
        (76, (RETAINED as u32 + 1).to_le_bytes()),
        (80, (FEATURES | 1 << 3).to_le_bytes()),
        (84, (RECORD_BYTES as u32 - 1).to_le_bytes()),
    ];
    for (offset, value) in mutations {
        let mut b = Header7::initial(LINEAGE).encode().unwrap();
        b[offset..offset + 4].copy_from_slice(&value);
        seal_header(&mut b);
        assert_eq!(Header7::decode(&b), Err(Error::Corrupt), "offset {offset}");
    }
    let mut b = Header7::initial(LINEAGE).encode().unwrap();
    b[88] = MAX_EXTENTS as u8 + 1;
    seal_header(&mut b);
    assert_eq!(Header7::decode(&b), Err(Error::Corrupt));
    let mut b = Header7::initial(LINEAGE).encode().unwrap();
    b[10] = 2;
    seal_header(&mut b);
    assert_eq!(Header7::decode(&b), Err(Error::Corrupt));
}

#[test]
fn header_invariants_are_checked_and_the_watermark_can_be_exhausted() {
    let base = Header7::initial(LINEAGE);
    assert!(base.validate().is_ok());
    assert_eq!(base.next_id(), Ok(Header7::NEXT_MIN));
    for broken in [
        Header7 { epoch: 10, ..base },
        Header7 { epoch: 0, ..base },
        Header7 {
            sequence: 0,
            ..base
        },
        Header7 {
            lineage: [0; 16],
            ..base
        },
        Header7 {
            next: Header7::NEXT_MIN - 1,
            ..base
        },
        Header7 {
            generation: 2,
            ..base
        },
    ] {
        assert_eq!(broken.validate(), Err(Error::Corrupt));
        assert_eq!(broken.encode(), Err(Error::Corrupt));
    }
    // The same head written by a peer that ignored an invariant is refused on the
    // way in, not trusted because its checksum is consistent.
    let mut b = base.encode().unwrap();
    b[28..36].copy_from_slice(&10u64.to_le_bytes());
    seal_header(&mut b);
    assert_eq!(Header7::decode(&b), Err(Error::Corrupt));
    // An exhausted watermark is still a valid head: it is the allocator that must
    // refuse, and it does so through the typed boundary.
    let exhausted = Header7 {
        next: Header7::NEXT_EXHAUSTED,
        ..base
    };
    assert!(exhausted.validate().is_ok());
    assert_eq!(exhausted.next_id(), Err(Error::Exhausted));
    assert_eq!(
        Header7::decode(&exhausted.encode().unwrap()).unwrap(),
        exhausted
    );
}

#[test]
fn the_node_offsets_and_checksum_are_fixed() {
    let b = node().encode().unwrap();
    assert_eq!(&b[0..4], &7u32.to_le_bytes());
    assert_eq!(&b[4..8], &5u32.to_le_bytes());
    assert_eq!(&b[8..16], &12u64.to_le_bytes());
    assert_eq!(&b[16..20], &200_000u32.to_le_bytes());
    assert_eq!(b[20], Kind::File as u8);
    assert_eq!(b[21], 1);
    assert_eq!(b[22], 3);
    assert_eq!(b[23], 3);
    // Eight (start, sectors) pairs, in file order.
    for (index, (start, sectors)) in [(0u32, 64u32), (200, 128), (1000, 199)].iter().enumerate() {
        let at = 24 + index * 8;
        assert_eq!(&b[at..at + 4], &start.to_le_bytes());
        assert_eq!(&b[at + 4..at + 8], &sectors.to_le_bytes());
    }
    assert_eq!(&b[24 + 3 * 8..88], &[0; 40]);
    assert_eq!(&b[88..91], b"app");
    assert!(b[91..120].iter().all(|byte| *byte == 0));
    assert_eq!(&b[120..124], &0xdead_beefu32.to_le_bytes());
    assert_eq!(
        u32::from_le_bytes(b[124..128].try_into().unwrap()),
        reference_crc(&b[..124])
    );
    assert_eq!(Node7::decode(&b).unwrap(), node());
    assert_eq!(Node7::decode(&b).unwrap().name(), b"app");
    assert_eq!(Node7::decode(&b).unwrap().runs().len(), 3);
}

#[test]
fn the_canonical_empty_node_is_zero_bytes_and_round_trips() {
    assert_eq!(Node7::EMPTY.encode(), Ok([0; NODE_BYTES]));
    assert_eq!(Node7::decode(&[0; NODE_BYTES]), Ok(Node7::EMPTY));
    assert!(Node7::EMPTY.validate().is_ok());
    assert!(Node7::decode(&[0; NODE_BYTES]).unwrap().runs().is_empty());
    // The empty slot has no CRC of its own: bytes that only carry a checksum are
    // neither the canonical slot nor a live record.
    let mut b = [0; NODE_BYTES];
    seal_node(&mut b);
    assert_eq!(Node7::decode(&b), Err(Error::Corrupt));
    // Kind 0 names nothing. A live record must be a file or a directory, so a
    // zero kind with an identity is corrupt even with a consistent checksum.
    let mut b = node().encode().unwrap();
    b[20] = 0;
    seal_node(&mut b);
    assert_eq!(Node7::decode(&b), Err(Error::Corrupt));
}

#[test]
fn a_node_refuses_corruption_and_malformed_records() {
    let mut b = node().encode().unwrap();
    b[16] ^= 1;
    assert_eq!(Node7::decode(&b), Err(Error::Corrupt));
    // Reserved and unused bytes are strict even when the checksum is recomputed.
    for offset in [91, 119, 24 + 3 * 8] {
        let mut b = node().encode().unwrap();
        b[offset] = 1;
        seal_node(&mut b);
        assert_eq!(Node7::decode(&b), Err(Error::Corrupt), "offset {offset}");
    }
    // Identity, space, name and kind.
    for broken in [
        Node7 { id: 0, ..node() },
        Node7 {
            version: 0,
            ..node()
        },
        Node7 { space: 0, ..node() },
        Node7 { space: 5, ..node() },
        Node7 {
            kind: Kind::Empty,
            ..node()
        },
        Node7 {
            name_length: 0,
            ..node()
        },
        Node7 {
            name_length: 32,
            ..node()
        },
        Node7 {
            name: *b"..\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            name_length: 2,
            ..node()
        },
        Node7 {
            name: *b"a/b\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            name_length: 3,
            ..node()
        },
    ] {
        assert_eq!(broken.encode(), Err(Error::Corrupt));
    }
    // A directory owns no payload at all.
    for broken in [
        Node7 {
            kind: Kind::Directory,
            ..node()
        },
        Node7 {
            kind: Kind::Directory,
            length: 0,
            payload_crc32: 1,
            extents_used: 0,
            extents: runs(&[]),
            ..node()
        },
        Node7 {
            kind: Kind::Directory,
            length: 0,
            payload_crc32: 0,
            extents: runs(&[]),
            ..node()
        },
    ] {
        assert_eq!(broken.encode(), Err(Error::Corrupt));
    }
    // A directory with no payload is the one valid directory shape.
    let empty_directory = Node7 {
        kind: Kind::Directory,
        length: 0,
        payload_crc32: 0,
        extents_used: 0,
        extents: runs(&[]),
        ..node()
    };
    assert!(empty_directory.validate().is_ok());
    assert_eq!(
        Node7::decode(&empty_directory.encode().unwrap()),
        Ok(empty_directory)
    );
}

#[test]
fn node_runs_must_be_exact_in_bounds_and_disjoint() {
    let valid = node();
    assert!(valid.validate().is_ok());
    for broken in [
        // Adjacent runs are one file, overlapping runs are not.
        Node7 {
            extents_used: 2,
            extents: runs(&[(0, 250), (200, 141)]),
            ..node()
        },
        // A used run with no sectors is not an extent.
        Node7 {
            extents_used: 3,
            extents: runs(&[(0, 64), (200, 0), (1000, 327)]),
            ..node()
        },
        // Past the payload region.
        Node7 {
            extents_used: 1,
            extents: runs(&[(DATA_SECTORS - 100, 200)]),
            ..node()
        },
        // More runs than the record holds.
        Node7 {
            extents_used: MAX_EXTENTS as u8 + 1,
            ..node()
        },
        // Unused run slots must be zero.
        Node7 {
            extents_used: 2,
            extents: {
                let mut list = runs(&[(0, 64), (200, 327)]);
                list[3] = Extent::new(9, 9);
                list
            },
            ..node()
        },
        // Fewer sectors than the length needs, and more.
        Node7 {
            extents_used: 1,
            extents: runs(&[(0, 390)]),
            ..node()
        },
        Node7 {
            extents_used: 1,
            extents: runs(&[(0, 392)]),
            ..node()
        },
    ] {
        assert_eq!(broken.encode(), Err(Error::Corrupt));
    }
    // Adjacent runs that still total the needed sectors are valid.
    let split = Node7 {
        extents_used: 2,
        extents: runs(&[(0, 64), (64, 327)]),
        ..node()
    };
    assert!(split.validate().is_ok());
}

#[test]
fn node_payload_geometry_is_exact_at_the_64k_and_256k_boundaries() {
    let file = |length, extents, used| Node7 {
        length,
        extents_used: used,
        extents,
        ..node()
    };
    // 64 KiB is 128 sectors: one byte more needs 129, and the record cannot claim
    // the smaller run list.
    assert!(file(64 * 1024, runs(&[(0, 128)]), 1).validate().is_ok());
    assert!(file(64 * 1024 + 1, runs(&[(0, 129)]), 1).validate().is_ok());
    assert_eq!(
        file(64 * 1024 + 1, runs(&[(0, 128)]), 1).validate(),
        Err(Error::Corrupt)
    );
    // 256 KiB is the largest file, and one byte more is refused outright.
    assert!(
        file(MAX_FILE_BYTES, runs(&[(0, 512)]), 1)
            .validate()
            .is_ok()
    );
    assert_eq!(
        file(MAX_FILE_BYTES + 1, runs(&[(0, 513)]), 1).validate(),
        Err(Error::Corrupt)
    );
    assert_eq!(
        file(MAX_FILE_BYTES + 1, runs(&[]), 0).validate(),
        Err(Error::Corrupt)
    );
    // An empty file is a file with no runs; the same run list with zero length
    // may not carry a run.
    assert!(file(0, runs(&[]), 0).validate().is_ok());
    assert_eq!(file(0, runs(&[(0, 1)]), 1).validate(), Err(Error::Corrupt));
    // The rule records use is the same one, reached through their own API.
    let retained = |length, extents| Record7 {
        length,
        extents_used: 1,
        extents,
        ..direct_committed()
    };
    assert!(retained(512, runs(&[(0, 1)])).validate(9, 100).is_ok());
    assert_eq!(
        retained(513, runs(&[(0, 1)])).validate(9, 100),
        Err(Error::Corrupt)
    );
}

#[test]
fn the_record_offsets_and_checksum_are_fixed() {
    let record = direct_committed();
    let b = record.encode().unwrap();
    assert_eq!(&b[0..8], &9u64.to_le_bytes());
    assert_eq!(&b[8..12], &3u32.to_le_bytes());
    assert_eq!(&b[12..16], &7u32.to_le_bytes());
    assert_eq!(&b[16..24], &4u64.to_le_bytes());
    assert_eq!(&b[24..32], &2u64.to_le_bytes());
    assert_eq!(&b[32..40], &11u64.to_le_bytes());
    assert_eq!(&b[40..48], &3u64.to_le_bytes());
    assert_eq!(&b[48..56], &4u64.to_le_bytes());
    assert_eq!(&b[56..64], &0u64.to_le_bytes());
    assert_eq!(&b[64..72], &4u64.to_le_bytes());
    assert_eq!(&b[72..76], &200_000u32.to_le_bytes());
    assert_eq!(&b[76..80], &0xfeed_faceu32.to_le_bytes());
    assert_eq!(b[80], RecordState::DirectCommitted as u8);
    assert_eq!(b[81], 0);
    assert_eq!(b[82], 3);
    assert_eq!(&b[83..88], &[0; 5]);
    for (index, (start, sectors)) in [(0u32, 64u32), (200, 128), (1000, 199)].iter().enumerate() {
        let at = 88 + index * 8;
        assert_eq!(&b[at..at + 4], &start.to_le_bytes());
        assert_eq!(&b[at + 4..at + 8], &sectors.to_le_bytes());
    }
    assert_eq!(&b[88 + 3 * 8..152], &[0; 40]);
    assert!(b[152..188].iter().all(|byte| *byte == 0));
    assert_eq!(
        u32::from_le_bytes(b[188..192].try_into().unwrap()),
        reference_crc(&b[..188])
    );
    let decoded = Record7::decode(&b).unwrap();
    assert_eq!(decoded, record);
    assert_eq!(decoded.runs().len(), 3);
    assert!(decoded.validate(9, 100).is_ok());
}

#[test]
fn every_record_state_round_trips_with_its_own_arithmetic() {
    for record in [
        direct_committed(),
        admitted(),
        cancelled(),
        admitted_committed(),
    ] {
        let b = record.encode().unwrap();
        assert_eq!(Record7::decode(&b), Ok(record));
        assert!(record.validate(9, 100).is_ok());
    }
    // Each state's defining equality is enforced, not assumed from the state
    // byte. Every refusal below starts from that state's own valid encoding and
    // changes exactly one field inside it, re-sealed with the independent CRC: a
    // record that differs from a decodable fixture only in the guarded field must
    // be refused for that field.
    let direct_bytes = direct_committed().encode().unwrap();
    assert_eq!(Record7::decode(&direct_bytes), Ok(direct_committed()));
    for tampered in [
        // A direct commit carries no admission.
        tamper(&direct_bytes, 56, &1u64.to_le_bytes()),
        // Direct commits commit past the metadata version they follow.
        tamper(&direct_bytes, 48, &3u64.to_le_bytes()),
        tamper(&direct_bytes, 40, &4u64.to_le_bytes()),
        // The instance belongs to the committed sequence.
        tamper(&direct_bytes, 16, &5u64.to_le_bytes()),
    ] {
        assert_eq!(Record7::decode(&tampered), Err(Error::Corrupt));
    }

    let admitted_bytes = admitted().encode().unwrap();
    assert_eq!(Record7::decode(&admitted_bytes), Ok(admitted()));
    for tampered in [
        // An admitted record has committed and terminal at zero.
        tamper(&admitted_bytes, 48, &5u64.to_le_bytes()),
        tamper(&admitted_bytes, 64, &5u64.to_le_bytes()),
        // Admission must follow the metadata version it replaces.
        tamper(&admitted_bytes, 56, &3u64.to_le_bytes()),
        // The instance belongs to the admission.
        tamper(&admitted_bytes, 16, &5u64.to_le_bytes()),
    ] {
        assert_eq!(Record7::decode(&tampered), Err(Error::Corrupt));
    }

    let cancelled_bytes = cancelled().encode().unwrap();
    assert_eq!(Record7::decode(&cancelled_bytes), Ok(cancelled()));
    for tampered in [
        // A cancelled record commits nothing and its terminal follows admission.
        tamper(&cancelled_bytes, 48, &7u64.to_le_bytes()),
        tamper(&cancelled_bytes, 64, &4u64.to_le_bytes()),
        tamper(&cancelled_bytes, 56, &3u64.to_le_bytes()),
        tamper(&cancelled_bytes, 16, &5u64.to_le_bytes()),
    ] {
        assert_eq!(Record7::decode(&tampered), Err(Error::Corrupt));
    }

    let committed_bytes = admitted_committed().encode().unwrap();
    assert_eq!(Record7::decode(&committed_bytes), Ok(admitted_committed()));
    for tampered in [
        // An admitted commit's terminal sequence is the committed one, and both
        // follow the admission.
        tamper(&committed_bytes, 64, &6u64.to_le_bytes()),
        tamper(&committed_bytes, 56, &5u64.to_le_bytes()),
        tamper(&committed_bytes, 40, &3u64.to_le_bytes()),
        tamper(&committed_bytes, 16, &4u64.to_le_bytes()),
    ] {
        assert_eq!(Record7::decode(&tampered), Err(Error::Corrupt));
    }

    // The struct-level path refuses the same shapes, so a writer cannot encode
    // what a reader would reject.
    for broken in [
        Record7 {
            admission_number: 1,
            ..direct_committed()
        },
        Record7 {
            committed: 3,
            terminal: 3,
            ..direct_committed()
        },
        Record7 {
            terminal: 3,
            ..direct_committed()
        },
        Record7 {
            committed: 5,
            ..admitted()
        },
        Record7 {
            terminal: 5,
            ..admitted()
        },
        Record7 {
            admission_number: 3,
            ..admitted()
        },
        Record7 {
            committed: 7,
            ..cancelled()
        },
        Record7 {
            terminal: 4,
            ..cancelled()
        },
        Record7 {
            terminal: 6,
            ..admitted_committed()
        },
        Record7 {
            admission_number: 5,
            ..admitted_committed()
        },
    ] {
        assert_eq!(broken.encode(), Err(Error::Corrupt));
    }
}

#[test]
fn a_replaced_metadata_version_is_never_zero() {
    // Every state replaces a metadata version it actually had: zero names no
    // version, which is the rule the v5 recovery codec and the ABI version type
    // already keep, so v7 refuses it locally in encode and in decode.
    for record in [
        direct_committed(),
        admitted(),
        cancelled(),
        admitted_committed(),
    ] {
        let fixture = record.encode().unwrap();
        assert_eq!(Record7::decode(&fixture), Ok(record));
        // The replaced version really is read: the same fixture with the smallest
        // real version decodes as that record.
        assert_eq!(
            Record7::decode(&tamper(&fixture, 40, &1u64.to_le_bytes())),
            Ok(Record7 {
                previous: 1,
                ..record
            })
        );
        // Zero is refused through the bytes and through the struct.
        assert_eq!(
            Record7::decode(&tamper(&fixture, 40, &0u64.to_le_bytes())),
            Err(Error::Corrupt),
            "{:?}",
            record.state
        );
        assert_eq!(
            Record7 {
                previous: 0,
                ..record
            }
            .encode(),
            Err(Error::Corrupt),
            "{:?}",
            record.state
        );
        // The contextual path refuses it too, and the boundary value passes there
        // as well.
        assert_eq!(
            Record7 {
                previous: 0,
                ..record
            }
            .validate(9, 100),
            Err(Error::Corrupt),
            "{:?}",
            record.state
        );
        assert!(
            Record7 {
                previous: 1,
                ..record
            }
            .validate(9, 100)
            .is_ok(),
            "{:?}",
            record.state
        );
    }
    // A zero version is not rescued by a state that would otherwise be legal: the
    // guard sits with the scope checks, not with the state arithmetic.
    assert_eq!(
        direct_committed().validate(9, 100),
        Ok(()),
        "the fixture itself stays valid"
    );
}

#[test]
fn a_direct_commit_terminal_sequence_is_the_committed_one() {
    let fixture = direct_committed().encode().unwrap();
    assert_eq!(Record7::decode(&fixture), Ok(direct_committed()));
    // A terminal sequence away from the committed one is refused in both
    // directions, including zero and the top of the domain, and re-sealing the
    // CRC changes nothing: the guard decides. Putting the field back restores a
    // decodable record, so the refusal is this field and not the fixture.
    for terminal in [0u64, 3, 5, u64::MAX] {
        assert_eq!(
            Record7::decode(&tamper(&fixture, 64, &terminal.to_le_bytes())),
            Err(Error::Corrupt),
            "terminal {terminal}"
        );
    }
    let restored = tamper(
        &tamper(&fixture, 64, &5u64.to_le_bytes()),
        64,
        &4u64.to_le_bytes(),
    );
    assert_eq!(Record7::decode(&restored), Ok(direct_committed()));
    // The struct path refuses the same shapes before any byte is written.
    assert_eq!(
        Record7 {
            terminal: 3,
            ..direct_committed()
        }
        .encode(),
        Err(Error::Corrupt)
    );
    // A later legal commit still round-trips, so the guard is not refusing every
    // commit that has moved past the fixture's sequence.
    let later = Record7 {
        committed: 5,
        terminal: 5,
        instance: 5,
        ..direct_committed()
    };
    assert_eq!(Record7::decode(&later.encode().unwrap()), Ok(later));
    assert!(later.validate(9, 100).is_ok());
}

#[test]
fn an_epoch_is_bounded_to_the_operation_that_created_it() {
    // The volume sequence is far ahead in every case, so only the local bound can
    // refuse: a direct commit answers on its committed sequence, and each
    // admission state answers on its admission number.
    let direct = direct_committed();
    assert_eq!(
        Record7 {
            retry_epoch: direct.committed,
            ..direct
        }
        .validate(100, 100),
        Ok(())
    );
    assert_eq!(
        Record7 {
            retry_epoch: direct.committed + 1,
            ..direct
        }
        .validate(100, 100),
        Err(Error::Corrupt)
    );
    for record in [admitted(), cancelled(), admitted_committed()] {
        let at_admission = Record7 {
            retry_epoch: record.admission_number,
            ..record
        };
        assert_eq!(at_admission.validate(100, 100), Ok(()));
        assert_eq!(
            Record7 {
                retry_epoch: record.admission_number + 1,
                ..record
            }
            .validate(100, 100),
            Err(Error::Corrupt)
        );
        // The same boundary holds in the bytes: the untouched fixture decodes,
        // the boundary value decodes as that fixture, and one past it does not.
        let fixture = record.encode().unwrap();
        assert_eq!(Record7::decode(&fixture), Ok(record));
        assert_eq!(
            Record7::decode(&tamper(
                &fixture,
                24,
                &record.admission_number.to_le_bytes()
            )),
            Ok(at_admission)
        );
        assert_eq!(
            Record7::decode(&tamper(
                &fixture,
                24,
                &(record.admission_number + 1).to_le_bytes()
            )),
            Err(Error::Corrupt)
        );
    }
    let direct_fixture = direct.encode().unwrap();
    assert_eq!(
        Record7::decode(&tamper(
            &direct_fixture,
            24,
            &direct.committed.to_le_bytes()
        )),
        Ok(Record7 {
            retry_epoch: direct.committed,
            ..direct
        })
    );
    assert_eq!(
        Record7::decode(&tamper(
            &direct_fixture,
            24,
            &(direct.committed + 1).to_le_bytes()
        )),
        Err(Error::Corrupt)
    );
}

#[test]
fn prevention_is_preserved_exactly_where_it_belongs() {
    for cause in [
        PreventionReason::Unknown,
        PreventionReason::Requested,
        PreventionReason::VersionConflict,
        PreventionReason::AuthorityLost,
    ] {
        let record = Record7 {
            prevention: Some(cause),
            ..cancelled()
        };
        let b = record.encode().unwrap();
        assert_eq!(b[81], cause as u8);
        assert_eq!(Record7::decode(&b), Ok(record));
    }
    // A cause byte outside the four known ones is corrupt.
    let mut b = cancelled().encode().unwrap();
    b[81] = 4;
    seal_record(&mut b);
    assert_eq!(Record7::decode(&b), Err(Error::Corrupt));
    // A cancellation without a cause is not "unknown", it is malformed.
    assert_eq!(
        Record7 {
            prevention: None,
            ..cancelled()
        }
        .encode(),
        Err(Error::Corrupt)
    );
    // The cause byte really is read: the same valid cancellation with a different
    // known cause decodes to that cause.
    let cancelled_bytes = cancelled().encode().unwrap();
    assert_eq!(
        Record7::decode(&tamper(
            &cancelled_bytes,
            81,
            &[PreventionReason::AuthorityLost as u8]
        )),
        Ok(Record7 {
            prevention: Some(PreventionReason::AuthorityLost),
            ..cancelled()
        })
    );
    // No other state may carry one, asserted from that state's own valid encoding
    // with only the cause byte changed and the CRC re-sealed. Byte 0 is the only
    // legal value outside a cancellation, so each case uses a real cause.
    for (fixture, cause) in [
        (
            direct_committed().encode().unwrap(),
            PreventionReason::Requested,
        ),
        (
            admitted().encode().unwrap(),
            PreventionReason::VersionConflict,
        ),
        (
            admitted_committed().encode().unwrap(),
            PreventionReason::AuthorityLost,
        ),
    ] {
        assert!(Record7::decode(&fixture).is_ok());
        assert_eq!(
            Record7::decode(&tamper(&fixture, 81, &[cause as u8])),
            Err(Error::Corrupt)
        );
    }
    for broken in [
        Record7 {
            prevention: Some(PreventionReason::Unknown),
            ..direct_committed()
        },
        Record7 {
            prevention: Some(PreventionReason::Requested),
            ..admitted()
        },
        Record7 {
            prevention: Some(PreventionReason::AuthorityLost),
            ..admitted_committed()
        },
    ] {
        assert_eq!(broken.encode(), Err(Error::Corrupt));
    }
}

#[test]
fn record_reserved_bytes_are_strict_even_with_a_valid_checksum() {
    for offset in [83, 84, 87, 152, 170, 187] {
        let mut b = direct_committed().encode().unwrap();
        b[offset] = 1;
        seal_record(&mut b);
        assert_eq!(Record7::decode(&b), Err(Error::Corrupt), "offset {offset}");
    }
    // Unused run slots and the state/scope bytes are strict too.
    for (offset, value) in [(88 + 3 * 8, 1u8), (80, 4u8), (82, MAX_EXTENTS as u8 + 1)] {
        let mut b = direct_committed().encode().unwrap();
        b[offset] = value;
        seal_record(&mut b);
        assert_eq!(Record7::decode(&b), Err(Error::Corrupt), "offset {offset}");
    }
    let mut b = direct_committed().encode().unwrap();
    b[32..40].fill(0);
    seal_record(&mut b);
    assert_eq!(Record7::decode(&b), Err(Error::Corrupt));
}

#[test]
fn an_empty_record_slot_is_not_a_record() {
    assert_eq!(Record7::decode(&[0; RECORD_BYTES]), Err(Error::Corrupt));
    assert_eq!(Record7::slot(&[0; RECORD_BYTES]), Ok(None));
    // Zero bytes with a valid-looking checksum are still not a record.
    let mut zeroed = [0; RECORD_BYTES];
    seal_record(&mut zeroed);
    assert_eq!(Record7::decode(&zeroed), Err(Error::Corrupt));
    let record = cancelled();
    let b = record.encode().unwrap();
    assert_eq!(Record7::slot(&b), Ok(Some(record)));
    // A whole receipt block: eight slots, then 512 reserved zero bytes.
    let block = [0; RECEIPT_BLOCK_BYTES];
    assert_eq!(receipt_slots(&block), Ok([None; RETAINED]));
    let mut block = [0; RECEIPT_BLOCK_BYTES];
    block[3 * RECORD_BYTES..4 * RECORD_BYTES].copy_from_slice(&b);
    let slots = receipt_slots(&block).unwrap();
    assert_eq!(slots[3], Some(record));
    assert!(
        slots
            .iter()
            .enumerate()
            .all(|(index, slot)| index == 3 || slot.is_none())
    );
    // The padding after the slots is reserved.
    let mut block = [0; RECEIPT_BLOCK_BYTES];
    block[RECORDS_BYTES] = 1;
    assert_eq!(receipt_slots(&block), Err(Error::Corrupt));
    assert_eq!(RECEIPT_RESERVED_BYTES, 512);
}

#[test]
fn record_identities_are_watermark_bounded_and_never_slot_bounded() {
    // Identities above the 256 record slots are ordinary identities.
    let far = Record7 {
        workspace: 300,
        object: 1000,
        ..direct_committed()
    };
    assert!(far.validate(9, 1001).is_ok());
    assert!(far.encode().is_ok());
    // The watermark is the bound: an identity at or above it is not allocated.
    assert_eq!(far.validate(9, 1000), Err(Error::Corrupt));
    assert!(Record7 { object: 999, ..far }.validate(9, 1000).is_ok());
    assert_eq!(
        Record7 {
            workspace: 1000,
            ..far
        }
        .validate(9, 1000),
        Err(Error::Corrupt)
    );
    // The record table holds 256 live nodes and the watermark does not: the same
    // record with an identity far above the table is bound by `next` alone.
    assert_eq!(NODES, 256);
    assert!(
        Record7 {
            object: 100_000,
            ..direct_committed()
        }
        .validate(9, 100_001)
        .is_ok()
    );
}

#[test]
fn record_scope_sequence_and_watermark_bounds_are_checked() {
    let record = direct_committed();
    assert!(record.validate(9, 100).is_ok());
    // Scope: a record names a subject, a workspace, an object above the reserved
    // roots, and an instance, and never its own workspace as an object.
    for broken in [
        Record7 {
            subject: 0,
            ..record
        },
        Record7 {
            workspace: 0,
            ..record
        },
        Record7 {
            instance: 0,
            ..record
        },
        Record7 {
            object: 4,
            ..record
        },
        Record7 {
            object: record.workspace,
            ..record
        },
        Record7 {
            retry_key: 0,
            ..record
        },
        Record7 {
            retry_epoch: 0,
            ..record
        },
    ] {
        assert_eq!(broken.encode(), Err(Error::Corrupt));
    }
    // The workspace is bounded by the watermark too.
    assert_eq!(
        Record7 {
            workspace: 100,
            ..record
        }
        .validate(9, 100),
        Err(Error::Corrupt)
    );
    // A watermark below the reserved roots is not a v7 volume.
    assert_eq!(
        record.validate(9, Header7::NEXT_MIN - 1),
        Err(Error::Corrupt)
    );
    assert_eq!(record.validate(0, 100), Err(Error::Corrupt));
    // The retry epoch belongs to the operation that created it, so a direct
    // commit answers on its committed sequence even when the volume sequence is
    // far ahead; the targeted epoch test covers every state.
    assert_eq!(
        Record7 {
            retry_epoch: record.committed,
            ..record
        }
        .validate(100, 100),
        Ok(())
    );
    assert_eq!(
        Record7 {
            retry_epoch: record.committed + 1,
            ..record
        }
        .validate(100, 100),
        Err(Error::Corrupt)
    );
    // The version-domain numbers are bounded by the volume sequence.
    assert_eq!(
        Record7 {
            committed: 10,
            terminal: 10,
            ..record
        }
        .validate(9, 100),
        Err(Error::Corrupt)
    );
    assert_eq!(
        Record7 {
            terminal: 10,
            ..cancelled()
        }
        .validate(9, 100),
        Err(Error::Corrupt)
    );
    assert_eq!(
        Record7 {
            admission_number: 10,
            ..admitted()
        }
        .validate(9, 100),
        Err(Error::Corrupt)
    );
    assert_eq!(
        Record7 {
            previous: 10,
            ..record
        }
        .validate(9, 100),
        Err(Error::Corrupt)
    );
}

#[test]
fn record_sequence_and_watermark_boundaries_hold_at_the_edges() {
    // The largest sequence and an exhausted watermark are both valid context; the
    // record's own numbers are then bounded by the u64 domain.
    let top = Record7 {
        previous: u64::MAX - 2,
        committed: u64::MAX - 1,
        terminal: u64::MAX - 1,
        instance: u64::MAX - 1,
        // The epoch is bounded by the commit it belongs to, not by the volume.
        retry_epoch: u64::MAX - 1,
        ..direct_committed()
    };
    assert!(top.validate(u64::MAX, Header7::NEXT_EXHAUSTED).is_ok());
    let b = top.encode().unwrap();
    assert_eq!(Record7::decode(&b), Ok(top));
    // An epoch above the committed sequence is refused before any context exists.
    assert_eq!(
        Record7 {
            retry_epoch: u64::MAX,
            ..top
        }
        .encode(),
        Err(Error::Corrupt)
    );
    // A volume sequence below the record's own versions is refused.
    assert_eq!(
        top.validate(u64::MAX - 2, Header7::NEXT_EXHAUSTED),
        Err(Error::Corrupt)
    );
    // A u32 watermark at its maximum cannot allocate; the record above it is out
    // of bounds rather than wrapping into the allocatable range.
    let high = Record7 {
        object: Header7::NEXT_EXHAUSTED,
        ..direct_committed()
    };
    assert_eq!(
        high.validate(9, Header7::NEXT_EXHAUSTED),
        Err(Error::Corrupt)
    );
}

#[test]
fn record_payload_geometry_matches_the_node_rule() {
    let record = direct_committed();
    assert!(record.validate(9, 100).is_ok());
    for broken in [
        // Fewer and more sectors than the length needs.
        Record7 {
            extents_used: 3,
            extents: runs(&[(0, 64), (200, 128), (1000, 198)]),
            ..record
        },
        Record7 {
            extents_used: 3,
            extents: runs(&[(0, 64), (200, 128), (1000, 200)]),
            ..record
        },
        // Overlapping runs.
        Record7 {
            extents_used: 2,
            extents: runs(&[(0, 200), (100, 191)]),
            ..record
        },
        // Past the payload region.
        Record7 {
            extents_used: 1,
            extents: runs(&[(DATA_SECTORS - 1, 2)]),
            ..record
        },
        // Unused slots must be zero.
        Record7 {
            extents: {
                let mut list = runs(&[(0, 64), (200, 128), (1000, 199)]);
                list[4] = Extent::new(1, 1);
                list
            },
            ..record
        },
        // A zero length may not carry a run, and the file limit still holds.
        Record7 {
            length: 0,
            ..record
        },
        Record7 {
            length: MAX_FILE_BYTES + 1,
            extents_used: 1,
            extents: runs(&[(0, 513)]),
            ..record
        },
    ] {
        assert_eq!(broken.encode(), Err(Error::Corrupt));
        assert_eq!(broken.validate(9, 100), Err(Error::Corrupt));
    }
    // The retained snapshot at the largest file boundary is valid.
    let largest = Record7 {
        length: MAX_FILE_BYTES,
        extents_used: 1,
        extents: runs(&[(0, 512)]),
        ..record
    };
    assert!(largest.validate(u64::MAX, 100).is_ok());
    let b = largest.encode().unwrap();
    assert_eq!(Record7::decode(&b), Ok(largest));
}

#[test]
fn aggregates_are_the_crc_of_the_region_they_name() {
    let nodes = [0xabu8; 4096];
    assert_eq!(aggregate(&nodes), reference_crc(&nodes));
    let mut changed = nodes;
    changed[1024] ^= 1;
    assert_ne!(aggregate(&nodes), aggregate(&changed));
    // The header carries whatever aggregate the writer measured, so a changed
    // region is detectable by the later stage that recomputes it.
    let header = Header7 {
        nodes_checksum: aggregate(&nodes),
        map_checksum: aggregate(&[]),
        receipts_checksum: aggregate(&[0; 64]),
        ..Header7::initial(LINEAGE)
    };
    let b = header.encode().unwrap();
    let decoded = Header7::decode(&b).unwrap();
    assert_eq!(decoded.nodes_checksum, aggregate(&nodes));
    assert_ne!(decoded.nodes_checksum, aggregate(&changed));
}
