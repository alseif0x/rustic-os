// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{
    Error, Packet, READ_CHUNK, READ_OPEN, REFERENCES,
    read::{Header, Info, MAX_INTEGER, Request},
    reference::{Epoch, References, Resource, Version, Workspace},
};

fn request() -> Request {
    let refs = References::new([0xab; 16], 4, 42).unwrap();
    Request {
        workspace: refs.workspace,
        resource: refs.resource,
        expected_version: Some(Version::new(9).unwrap()),
        offset: 13,
        length: 51,
    }
}

#[test]
fn stable_references_have_canonical_bounded_text() {
    let refs = References::new([0xab; 16], 4, 42).unwrap();
    let workspace = "ws_abababababababababababababababab_00000004";
    let resource = "rs_abababababababababababababababab_00000004_0000002a";
    assert_eq!(refs.workspace.to_string(), workspace);
    assert_eq!(refs.resource.to_string(), resource);
    assert_eq!(workspace.len(), 44);
    assert_eq!(resource.len(), 53);
    assert_eq!(workspace.parse(), Ok(refs.workspace));
    assert_eq!(resource.parse(), Ok(refs.resource));
    let version = Version::new(u64::MAX).unwrap();
    assert_eq!(version.to_string(), "v_ffffffffffffffff");
    assert_eq!(version.to_string().parse(), Ok(version));
    let epoch = Epoch::new(1).unwrap();
    assert_eq!(epoch.to_string(), "e_0000000000000001");
    assert_eq!(epoch.to_string().parse(), Ok(epoch));
}

#[test]
fn invalid_identity_encodings_do_not_alias_valid_references() {
    let refs = References::new([0xab; 16], 4, 42).unwrap();
    for token in [
        refs.workspace.to_string(),
        refs.resource.to_string(),
        Version::new(9).unwrap().to_string(),
        Epoch::new(3).unwrap().to_string(),
    ] {
        for invalid in [
            token.to_uppercase(),
            format!("{token} "),
            format!(" {token}"),
            format!("{token}\n"),
            token[..token.len() - 1].to_owned(),
            token.replacen('_', "/", 1),
            token.replace('0', "é"),
        ] {
            assert!(invalid.parse::<Workspace>().is_err());
            assert!(invalid.parse::<Resource>().is_err());
            assert!(invalid.parse::<Version>().is_err());
            assert!(invalid.parse::<Epoch>().is_err());
        }
    }
    for invalid in [
        "v_0000000000000000",
        "e_0000000000000000",
        "v_000000000000000g",
    ] {
        assert!(invalid.parse::<Version>().is_err());
        assert!(invalid.parse::<Epoch>().is_err());
    }
    assert!(Workspace::new([0; 16], 4).is_err());
    assert!(Workspace::new([1; 16], 0).is_err());
    assert!(Resource::new(refs.workspace, 0).is_err());
}

#[test]
fn reference_bootstrap_has_no_subject_or_inspection_request() {
    let request = References::request(4, 42, 7).unwrap();
    assert_eq!(
        (request.op, request.id, request.arg, request.context),
        (REFERENCES, 42, 4, 7)
    );
    assert_eq!((request.count, request.version), (0, 0));
    let refs = References::new([0xab; 16], 4, 42).unwrap();
    let reply = refs.packet(7).unwrap();
    assert_eq!(
        References::decode(&Packet::decode(&reply.encode()).unwrap()),
        Ok(refs)
    );
    for bad in [
        Packet {
            version: 1,
            ..reply
        },
        Packet { count: 15, ..reply },
        Packet { id: 0, ..reply },
        Packet { status: 1, ..reply },
    ] {
        assert!(References::decode(&bad).is_err());
    }
}

#[test]
fn range_request_layout_separates_service_version_and_transport_context() {
    let request = request();
    let p = request.packet(READ_OPEN, 7).unwrap();
    let bytes = p.encode();
    assert_eq!(&bytes[..4], &[1, 16, 0, 30]);
    assert_eq!(&bytes[4..8], &42u32.to_le_bytes());
    assert_eq!(&bytes[8..12], &51u32.to_le_bytes());
    assert_eq!(&bytes[12..16], &7u32.to_le_bytes());
    assert_eq!(&bytes[16..24], &9u64.to_le_bytes());
    assert_eq!(&bytes[24..40], &[0xab; 16]);
    assert_eq!(&bytes[40..44], &4u32.to_le_bytes());
    assert_eq!(&bytes[44..52], &13u64.to_le_bytes());
    assert_eq!(&bytes[52..54], &[1, 0]);
    assert_eq!(&bytes[54..], &[0; 10]);
    assert_eq!(
        Request::decode(&Packet::decode(&bytes).unwrap()),
        Ok(request)
    );
    let mut future = p;
    future.data[28] = 2;
    assert_eq!(Request::decode(&future), Err(Error::UnsupportedVersion));
    let mut wrong_transport = bytes;
    wrong_transport[0] = 2;
    assert_eq!(Packet::decode(&wrong_transport), Err(Error::Protocol));
}

#[test]
fn native_ranges_reject_unsafe_bounds_mixed_workspaces_and_unpinned_chunks() {
    let base = request();
    let other = Workspace::new([0xcd; 16], 4).unwrap();
    for bad in [
        Request {
            workspace: other,
            ..base
        },
        Request { length: 0, ..base },
        Request {
            length: 1025,
            ..base
        },
        Request {
            offset: MAX_INTEGER,
            ..base
        },
        Request {
            offset: u64::MAX,
            ..base
        },
    ] {
        assert_eq!(bad.packet(READ_OPEN, 7), Err(Error::Invalid));
    }
    assert_eq!(base.packet(READ_CHUNK, 7), Err(Error::Invalid));
    let chunk = Request {
        expected_version: None,
        length: 40,
        ..base
    };
    assert_eq!(chunk.packet(READ_CHUNK, 7), Err(Error::Invalid));
    assert!(chunk.packet(READ_OPEN, 7).is_ok());
    let mut p = base.packet(READ_OPEN, 7).unwrap();
    p.arg = u32::MAX;
    assert_eq!(Request::decode(&p), Err(Error::Invalid));
    p = base.packet(READ_OPEN, 7).unwrap();
    p.data[30] = 1;
    assert_eq!(Request::decode(&p), Err(Error::Invalid));
}

#[test]
fn range_header_pins_version_and_derives_eof_without_inventing_progress() {
    let request = request();
    let header = Header {
        id: 42,
        size: 40,
        version: Version::new(9).unwrap(),
        range_sha256: [0x5a; 32],
        retry_epoch: Epoch::new(3).unwrap(),
    };
    let packet = header.packet(7).unwrap();
    assert_eq!(
        Header::decode(&Packet::decode(&packet.encode()).unwrap()),
        Ok(header)
    );
    assert_eq!(&packet.data[..32], &[0x5a; 32]);
    assert_eq!(&packet.data[32..], &3u64.to_le_bytes());
    let info = Info::from_header(request, header).unwrap();
    assert_eq!((info.offset, info.length, info.eof), (13, 27, true));
    let eof = Info::from_header(
        Request {
            offset: 40,
            ..request
        },
        header,
    )
    .unwrap();
    assert_eq!((eof.length, eof.eof), (0, true));
    assert!(
        Info::from_header(
            Request {
                offset: 41,
                ..request
            },
            header
        )
        .is_err()
    );
    assert!(
        Info::from_header(
            request,
            Header {
                version: Version::new(10).unwrap(),
                ..header
            }
        )
        .is_err()
    );
    let mut malformed = packet;
    malformed.data[32..].fill(0);
    assert!(Header::decode(&malformed).is_err());
}

#[test]
fn errors_must_be_complete_correlated_envelopes_without_residual_results() {
    let mut denied = Packet::new(READ_OPEN);
    denied.context = 7;
    denied.status = Error::Denied as u8;
    assert_eq!(denied.checked_reply(READ_OPEN, 7), Err(Error::Denied));
    for bad in [
        Packet { id: 42, ..denied },
        Packet { arg: 1, ..denied },
        Packet {
            version: 1,
            ..denied
        },
        Packet { count: 1, ..denied },
        Packet {
            context: 8,
            ..denied
        },
        Packet {
            op: READ_CHUNK,
            ..denied
        },
    ] {
        assert_eq!(bad.checked_reply(READ_OPEN, 7), Err(Error::Protocol));
    }
}
