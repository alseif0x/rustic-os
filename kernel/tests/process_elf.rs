// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::process::elf::{Error, Image};

fn field(bytes: &mut [u8], at: usize, value: u64, size: usize) {
    bytes[at..at + size].copy_from_slice(&value.to_le_bytes()[..size]);
}

fn image() -> Vec<u8> {
    let mut bytes = vec![0; 8200];
    bytes[..9].copy_from_slice(b"\x7fELF\x02\x01\x01\x00\x00");
    for (at, value, size) in [
        (16, 2, 2),
        (18, 62, 2),
        (20, 1, 4),
        (24, 0x400000, 8),
        (32, 64, 8),
        (52, 64, 2),
        (54, 56, 2),
        (56, 2, 2),
        (64, 1, 4),
        (68, 5, 4),
        (72, 4096, 8),
        (80, 0x400000, 8),
        (96, 16, 8),
        (104, 16, 8),
        (112, 4096, 8),
        (120, 1, 4),
        (124, 6, 4),
        (128, 8192, 8),
        (136, 0x600000, 8),
        (152, 8, 8),
        (160, 4096, 8),
        (168, 4096, 8),
    ] {
        field(&mut bytes, at, value, size);
    }
    bytes[4096] = 0xcc;
    bytes[8192] = 0x42;
    bytes
}

#[test]
fn parses_disjoint_code_data_and_zero_fill_extent() {
    let bytes = image();
    let elf = Image::parse(&bytes).unwrap();
    assert_eq!(elf.entry(), 0x400000);
    assert_eq!(elf.segments().len(), 2);
    let data = &elf.segments()[1];
    assert_eq!((data.file_size, data.memory_size), (8, 4096));
    assert!(data.writable && !data.executable);
    assert_eq!(elf.data(data)[0], 0x42);
}

#[test]
fn rejects_malformed_and_unsupported_inputs_before_loading() {
    for (at, value, size) in [
        (4, 1, 1),
        (5, 2, 1),
        (7, 3, 1),
        (16, 3, 2),
        (18, 3, 2),
        (20, 2, 4),
        (32, u64::MAX, 8),
        (52, 63, 2),
        (54, 55, 2),
        (56, 0, 2),
        (56, 17, 2),
        (64, 2, 4),
        (64, 3, 4),
        (64, 7, 4),
        (68, 7, 4),
        (68, 0, 4),
        (72, u64::MAX, 8),
        (80, 0, 8),
        (80, u64::MAX - 8, 8),
        (96, 17, 8),
        (104, 0, 8),
        (112, 3, 8),
        (136, 0x400000, 8),
        (136, 0x400800, 8),
        (136, 0x80000000, 8),
        (160, 1024 * 1024, 8),
        (24, 0x600000, 8),
        (24, 0x400010, 8),
    ] {
        let mut bytes = image();
        field(&mut bytes, at, value, size);
        assert!(Image::parse(&bytes).is_err(), "accepted field {at}={value}");
    }
    let bytes = image();
    for length in [0, 8, 63, 64, 119, 175, 4096, 8199] {
        assert!(
            Image::parse(&bytes[..length]).is_err(),
            "accepted truncated {length}"
        );
    }
}

#[test]
fn supports_non_page_aligned_segments_but_rejects_shared_pages() {
    let mut bytes = image();
    field(&mut bytes, 72, 4097, 8);
    field(&mut bytes, 80, 0x400001, 8);
    field(&mut bytes, 24, 0x400001, 8);
    assert!(Image::parse(&bytes).is_ok());
    field(&mut bytes, 136, 0x400020, 8);
    field(&mut bytes, 128, 0x1020, 8);
    assert!(matches!(Image::parse(&bytes), Err(Error::Overlap)));
}

#[test]
fn arbitrary_header_mutations_never_panic() {
    let original = image();
    for offset in 0..176 {
        for value in [0, 1, 127, 255] {
            let mut bytes = original.clone();
            bytes[offset] = value;
            let _ = Image::parse(&bytes);
        }
    }
}
