// SPDX-License-Identifier: Apache-2.0
use rustic_abi::application::{Error, Manifest, SIZE};
fn manifest() -> [u8; SIZE] {
    let mut bytes = [0; SIZE];
    bytes[..8].copy_from_slice(b"RUSTAPP\0");
    bytes[8..12].copy_from_slice(&[1, 0, 128, 0]);
    bytes[12..16].copy_from_slice(&65536u32.to_le_bytes());
    bytes[16] = 1;
    bytes[20] = 1;
    bytes[24] = 3;
    bytes[32..54].copy_from_slice(b"org.rusticos.sdk-probe");
    let executable = b"sdk-probe.elf";
    bytes[64..64 + executable.len()].copy_from_slice(executable);
    bytes
}
#[test]
fn representation_and_admission_do_not_grant_authority() {
    let bytes = manifest();
    let parsed = Manifest::parse(&bytes).unwrap();
    assert_eq!(parsed.identity, "org.rusticos.sdk-probe");
    assert_eq!(parsed.executable, "sdk-probe.elf");
    assert_eq!(parsed.version, [0, 1, 0]);
    assert!(!parsed.admitted(0));
    assert!(!parsed.admitted(1));
    assert!(parsed.admitted(3));
}
#[test]
fn rejects_truncation_extensions_versions_and_unknown_requests() {
    let bytes = manifest();
    for length in 0..SIZE {
        assert_eq!(
            Manifest::parse(&bytes[..length]).unwrap_err(),
            Error::Length
        );
    }
    assert_eq!(Manifest::parse(&[0; SIZE + 1]).unwrap_err(), Error::Length);
    for (offset, error) in [
        (8, Error::Version),
        (10, Error::Version),
        (12, Error::Abi),
        (16, Error::Ipc),
        (24, Error::Capabilities),
        (96, Error::Reserved),
    ] {
        let mut bad = bytes;
        bad[offset] = 255;
        assert_eq!(Manifest::parse(&bad).unwrap_err(), error);
    }
}
#[test]
fn rejects_paths_non_ascii_hidden_suffixes_and_unterminated_names() {
    for name in [
        b"../app.elf".as_slice(),
        b"/app.elf",
        b"A.elf",
        b"a\xff.elf",
        b"a\0hidden.elf",
    ] {
        let mut bytes = manifest();
        bytes[64..96].fill(0);
        bytes[64..64 + name.len()].copy_from_slice(name);
        assert_eq!(Manifest::parse(&bytes).unwrap_err(), Error::Executable);
    }
    let mut bytes = manifest();
    bytes[32..64].fill(b'a');
    assert_eq!(Manifest::parse(&bytes).unwrap_err(), Error::Identity);
}

#[test]
fn block_feature_is_admitted_independently_of_ipc() {
    use rustic_abi::application::{BLOCK, DIAGNOSTIC, IPC, KNOWN};
    let mut bytes = manifest();
    bytes[24..32].copy_from_slice(&(BLOCK | DIAGNOSTIC).to_le_bytes());
    let parsed = Manifest::parse(&bytes).unwrap();
    assert!(parsed.admitted(KNOWN));
    assert!(parsed.admitted(BLOCK | DIAGNOSTIC));
    assert!(!parsed.admitted(IPC | DIAGNOSTIC));
}
