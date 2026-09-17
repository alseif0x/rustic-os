// SPDX-License-Identifier: Apache-2.0
//! Host tests for the v6 control records and header (#51).
use rustic_fs::{
    DATA_BYTES_V6, DATA_SECTORS, Error, Extent, Header6, Kind, MAP_SECTORS, NODE_BYTES,
    NODES_SECTORS, Node6, OBJECTS_V6, PAYLOAD_SECTOR, VOLUME_SECTORS,
};

fn node() -> Node6 {
    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"app\0");
    Node6 {
        id: 7,
        parent: 4,
        version: 12,
        // Far beyond the 16-bit field the v5 record carries.
        length: 220_000,
        kind: Kind::File,
        space: 1,
        extents: {
            let mut runs = [Extent::new(0, 0); 8];
            runs[0] = Extent::new(0, 64);
            runs[1] = Extent::new(200, 128);
            runs[2] = Extent::new(1000, 248);
            runs
        },
        extents_used: 3,
        name,
        name_length: 4,
    }
}

#[test]
fn the_v6_geometry_is_the_selected_budget_and_fits_the_disk() {
    // Header, 256 nodes of 128 bytes, the 16 KiB map, then the 64 MiB payload.
    assert_eq!(NODE_BYTES, 128);
    assert_eq!(NODES_SECTORS, 64);
    assert_eq!(MAP_SECTORS, 32);
    assert_eq!(PAYLOAD_SECTOR, 8 + 1 + 64 + 32);
    assert_eq!(VOLUME_SECTORS, PAYLOAD_SECTOR + DATA_SECTORS);
    // The reference disk is 4 GiB; the volume structures plus payload fit well
    // inside it. Checked at compile time so a geometry change cannot pass by
    // accident.
    const { assert!(VOLUME_SECTORS * 512 <= 4 * 1024 * 1024 * 1024) };
    assert_eq!(OBJECTS_V6 * NODE_BYTES, 32 * 1024);
    assert_eq!(DATA_BYTES_V6, 64 * 1024 * 1024);
}

#[test]
fn a_control_record_round_trips_with_a_length_beyond_16_bits() {
    let value = node();
    let bytes = value.encode();
    assert_eq!(Node6::decode(&bytes), Ok(value));
    assert_eq!(value.runs().len(), 3);
    assert_eq!(value.runs()[1], Extent::new(200, 128));
    // The record is exactly one bounded size, so the node table has a known size.
    assert_eq!(bytes.len(), NODE_BYTES);
    assert_eq!(Node6::decode(&Node6::EMPTY.encode()), Ok(Node6::EMPTY));
}

#[test]
fn a_corrupt_or_impossible_record_is_refused() {
    let bytes = node().encode();
    // A flipped byte anywhere in the body fails the record checksum.
    let mut flipped = bytes;
    flipped[16] ^= 1;
    assert_eq!(Node6::decode(&flipped), Err(Error::Corrupt));
    // An unknown kind, a nonzero reserved byte and too many extents are refused.
    let mut bad_kind = bytes;
    bad_kind[20] = 3;
    let checksum = crc_of(&bad_kind);
    bad_kind[NODE_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    assert_eq!(Node6::decode(&bad_kind), Err(Error::Corrupt));

    let mut reserved = bytes;
    reserved[121] = 1;
    assert_eq!(Node6::decode(&reserved), Err(Error::Corrupt));

    let mut many = bytes;
    many[22] = 9;
    let checksum = crc_of(&many);
    many[NODE_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    assert_eq!(Node6::decode(&many), Err(Error::Corrupt));

    // A run that leaves the payload region is refused.
    let mut outside = bytes;
    outside[24..28].copy_from_slice(&(DATA_SECTORS as u32 - 1).to_le_bytes());
    outside[28..32].copy_from_slice(&2u32.to_le_bytes());
    let checksum = crc_of(&outside);
    outside[NODE_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    assert_eq!(Node6::decode(&outside), Err(Error::Corrupt));

    // A declared length larger than the runs can hold is refused.
    let mut long = bytes;
    long[16..20].copy_from_slice(&300_000u32.to_le_bytes());
    let checksum = crc_of(&long);
    long[NODE_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    assert_eq!(Node6::decode(&long), Err(Error::Corrupt));
}

#[test]
fn the_header_round_trips_and_rejects_a_wrong_magic_version_or_geometry() {
    let mut header = Header6::initial();
    header.nodes_checksum = 0x1122_3344;
    header.map_checksum = 0x5566_7788;
    let bytes = header.encode();
    assert_eq!(Header6::decode(&bytes), Ok(header));

    let mut wrong_magic = bytes;
    wrong_magic[0] = b'X';
    assert_eq!(Header6::decode(&wrong_magic), Err(Error::Corrupt));
    // A v5 header must never decode as v6.
    let mut v5 = bytes;
    v5[7] = b'1';
    assert_eq!(Header6::decode(&v5), Err(Error::Corrupt));

    let mut wrong_version = bytes;
    wrong_version[8] = 5;
    assert_eq!(Header6::decode(&wrong_version), Err(Error::Corrupt));

    let mut wrong_objects = bytes;
    wrong_objects[20..24].copy_from_slice(&32u32.to_le_bytes());
    assert_eq!(Header6::decode(&wrong_objects), Err(Error::Corrupt));

    let mut nonzero_tail = bytes;
    nonzero_tail[44] = 1;
    assert_eq!(Header6::decode(&nonzero_tail), Err(Error::Corrupt));
}

/// The record checksum covers every byte before the checksum field. This mirror
/// matches `crates/fs/src/checksum.rs` (reflected CRC-32, check value 0xCBF43926).
fn crc_of(bytes: &[u8; NODE_BYTES]) -> u32 {
    crate_crc(&bytes[..NODE_BYTES - 4])
}

fn crate_crc(bytes: &[u8]) -> u32 {
    let mut value = !0u32;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(value & 1)));
        }
    }
    !value
}
