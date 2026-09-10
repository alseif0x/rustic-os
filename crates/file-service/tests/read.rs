// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_abi::files::{
    read::{Header, MAX_INTEGER, Request},
    reference::{References, Version},
    *,
};
use rustic_file_service::{Grant, Server};
use rustic_fs::{Kind, Volume};
use support::{grant, request, run, setup};

fn references(workspace: u32, file: u32) -> References {
    References::new([7; 16], workspace, file).unwrap()
}

fn read(refs: References, offset: u64, length: u16, version: u64) -> Request {
    Request {
        workspace: refs.workspace,
        resource: refs.resource,
        expected_version: if version == 0 {
            None
        } else {
            Some(Version::new(version).unwrap())
        },
        offset,
        length,
    }
}

fn hash(value: &str) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    bytes
}

#[test]
fn read_only_helper_gets_identity_epoch_without_directory_or_receipt_authority() {
    let (mut server, mut disk, a, b) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let node = server.volume.stat(a).unwrap();
    server
        .volume
        .replace(&mut disk, a, node.version, b"Old content\n")
        .unwrap();
    let c = grant(&mut server, 0, a, 3, 0);
    let h = server
        .derive(
            1,
            c,
            Grant {
                peer: 11,
                endpoint: 2,
                scope: a,
                rights: READ_RIGHT,
                generation: 0,
                expires: 0,
                subject: 0,
            },
            0,
        )
        .unwrap();
    let found = run(
        &mut server,
        &mut disk,
        1,
        References::request(4, a, h).unwrap(),
        0,
    );
    assert_eq!(References::decode(&found).unwrap(), references(4, a));
    let opened = run(
        &mut server,
        &mut disk,
        1,
        read(references(4, a), 0, 1024, 0)
            .packet(READ_OPEN, h)
            .unwrap(),
        0,
    );
    let header = Header::decode(&opened).unwrap();
    assert_eq!(header.retry_epoch.value(), 1);
    assert_eq!(
        header.range_sha256,
        hash("d7fdb24d671e6157ebad50b2190a56717a05e72f57254197d89863a2048b80b3")
    );
    for packet in [
        request(LIST, 4, h),
        request(RECOVERY, a, h),
        request(BEGIN, a, h),
        References::request(4, b, h).unwrap(),
    ] {
        let denied = run(&mut server, &mut disk, 1, packet, 0);
        assert_eq!(denied.status, Error::Denied as u8);
        assert_eq!(denied.data, [0; DATA]);
    }
}

#[test]
fn wrong_workspace_lineage_and_missing_objects_have_generic_denials() {
    let (mut server, mut disk, a, b) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let context = grant(&mut server, 0, a, READ_RIGHT, 0);
    for refs in [
        references(2, a),
        references(a, a),
        references(u32::MAX, a),
        references(4, b),
        references(4, u32::MAX),
        References::new([8; 16], 4, a).unwrap(),
    ] {
        let denied = run(
            &mut server,
            &mut disk,
            0,
            read(refs, 0, 1, 0).packet(READ_OPEN, context).unwrap(),
            0,
        );
        assert_eq!(denied.status, Error::Denied as u8);
        assert_eq!(
            (denied.id, denied.arg, denied.version, denied.count),
            (0, 0, 0, 0)
        );
        assert_eq!(denied.data, [0; DATA]);
    }
    for workspace in [0, a, 2, u32::MAX] {
        let packet = Packet {
            arg: workspace,
            ..request(REFERENCES, a, context)
        };
        assert_eq!(
            run(&mut server, &mut disk, 0, packet, 0).status,
            Error::Denied as u8
        );
    }
}

#[test]
fn legacy_store_does_not_invent_identity_or_upgrade_on_read() {
    let (mut server, mut disk, a, _) = setup();
    let context = grant(&mut server, 0, a, READ_RIGHT, 0);
    let sequence = server.volume.sequence();
    for packet in [
        References::request(4, a, context).unwrap(),
        read(references(4, a), 0, 1, 0)
            .packet(READ_OPEN, context)
            .unwrap(),
    ] {
        assert_eq!(
            run(&mut server, &mut disk, 0, packet, 0).status,
            Error::Unavailable as u8
        );
    }
    assert_eq!(server.volume.sequence(), sequence);
    assert_eq!(
        server.volume.recovery_info(),
        Err(rustic_fs::Error::Unsupported)
    );
    assert_eq!(
        run(&mut server, &mut disk, 0, request(READ, a, context), 0).status,
        0
    );
}

#[test]
fn binary_ranges_hash_exact_bytes_and_do_not_allocate_transfer_slots() {
    let (mut server, mut disk, a, _) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let data: Vec<u8> = (0..1024).map(|index| index as u8).collect();
    let initial = server.volume.stat(a).unwrap();
    let file = server
        .volume
        .replace(&mut disk, a, initial.version, &data)
        .unwrap();
    let context = grant(&mut server, 0, a, READ_RIGHT, 0);
    for (offset, length, expected_hash) in [
        (
            0,
            1024,
            "785b0751fc2c53dc14a4ce3d800e69ef9ce1009eb327ccf458afe09c242c26c9",
        ),
        (
            39,
            41,
            "64d85b630f96c234b4e9447074559a4695e81cbadb6924432f52738895f7c43d",
        ),
        (
            1024,
            1,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
    ] {
        let opened = run(
            &mut server,
            &mut disk,
            0,
            read(references(4, a), offset, length, 0)
                .packet(READ_OPEN, context)
                .unwrap(),
            0,
        );
        let header = Header::decode(&opened).unwrap();
        assert_eq!((header.size, header.version.value()), (1024, file.version));
        assert_eq!(header.range_sha256, hash(expected_hash));
        let mut received = Vec::new();
        let count = usize::from(length).min(data.len() - offset as usize);
        while received.len() < count {
            let n = (count - received.len()).min(DATA) as u16;
            let chunk = read(
                references(4, a),
                offset + received.len() as u64,
                n,
                header.version.value(),
            )
            .packet(READ_CHUNK, context)
            .unwrap();
            let result = run(&mut server, &mut disk, 0, chunk, 0);
            assert_eq!(result.status, 0);
            assert_eq!(
                (result.id, result.arg, result.version),
                (a, 1024, file.version)
            );
            assert_eq!(result.count, n as u8);
            received.extend_from_slice(result.payload());
        }
        assert_eq!(received, data[offset as usize..offset as usize + count]);
        assert_eq!(server.pending(), 0);
    }
}

#[test]
fn version_change_between_header_and_chunks_is_never_a_mixed_success() {
    let (mut server, mut disk, a, _) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let initial = server.volume.stat(a).unwrap();
    let file = server
        .volume
        .replace(&mut disk, a, initial.version, &[0x31; 80])
        .unwrap();
    let context = grant(&mut server, 0, a, READ_RIGHT, 0);
    let opened = run(
        &mut server,
        &mut disk,
        0,
        read(references(4, a), 0, 80, 0)
            .packet(READ_OPEN, context)
            .unwrap(),
        0,
    );
    let header = Header::decode(&opened).unwrap();
    let first = read(references(4, a), 0, 40, header.version.value())
        .packet(READ_CHUNK, context)
        .unwrap();
    assert_eq!(
        run(&mut server, &mut disk, 0, first, 0).payload(),
        &[0x31; 40]
    );
    server
        .volume
        .replace(&mut disk, a, file.version, &[0x72; 80])
        .unwrap();
    for op in [READ_OPEN, READ_CHUNK] {
        let old = read(references(4, a), 40, 40, header.version.value())
            .packet(op, context)
            .unwrap();
        let result = run(&mut server, &mut disk, 0, old, 0);
        assert_eq!(result.status, Error::Version as u8);
        assert_eq!(result.data, [0; DATA]);
    }
}

#[test]
fn every_chunk_rechecks_peer_context_expiry_and_revocation() {
    let (mut server, mut disk, a, _) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let context = grant(&mut server, 0, a, READ_RIGHT, 10);
    let file = server.volume.stat(a).unwrap();
    let chunk = read(references(4, a), 0, 1, file.version)
        .packet(READ_CHUNK, context)
        .unwrap();
    assert_eq!(
        server.handle(&mut disk, 0, 99, chunk, 0).status,
        Error::Denied as u8
    );
    assert_eq!(
        run(&mut server, &mut disk, 0, chunk, 10).status,
        Error::Expired as u8
    );
    server.revoke(0).unwrap();
    assert_eq!(
        run(&mut server, &mut disk, 0, chunk, 0).status,
        Error::Revoked as u8
    );
    let fresh = grant(&mut server, 0, a, READ_RIGHT, 0);
    assert_ne!(fresh, context);
    assert_eq!(
        run(&mut server, &mut disk, 0, chunk, 0).status,
        Error::Revoked as u8
    );
    assert_eq!(
        run(
            &mut server,
            &mut disk,
            0,
            Packet {
                context: fresh,
                ..chunk
            },
            0
        )
        .status,
        0
    );
}

#[test]
fn remount_keeps_references_but_requires_new_authority_and_recreation_gets_new_ids() {
    let (mut server, mut disk, _, _) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let workspace = server
        .volume
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let file = server
        .volume
        .create(&mut disk, workspace.id, b"file", Kind::File)
        .unwrap();
    let context = grant(&mut server, 0, workspace.id, READ_RIGHT, 0);
    let packet = read(references(workspace.id, file.id), 0, 1, file.version)
        .packet(READ_OPEN, context)
        .unwrap();
    let before = run(&mut server, &mut disk, 0, packet, 0);
    let mut server = Server::new(Volume::mount(&mut disk).unwrap());
    assert_eq!(
        run(&mut server, &mut disk, 0, packet, 0).status,
        Error::Denied as u8
    );
    let fresh = grant(&mut server, 0, workspace.id, READ_RIGHT, 0);
    let resumed = run(
        &mut server,
        &mut disk,
        0,
        Packet {
            context: fresh,
            ..packet
        },
        0,
    );
    assert_eq!(
        Header::decode(&before).unwrap(),
        Header::decode(&resumed).unwrap()
    );
    server.volume.remove(&mut disk, file.id).unwrap();
    server.volume.remove(&mut disk, workspace.id).unwrap();
    let workspace2 = server
        .volume
        .create(&mut disk, 4, b"project", Kind::Directory)
        .unwrap();
    let file2 = server
        .volume
        .create(&mut disk, workspace2.id, b"file", Kind::File)
        .unwrap();
    assert!(workspace2.id > workspace.id && file2.id > file.id);
    let fresh = grant(&mut server, 0, workspace2.id, READ_RIGHT, 0);
    assert_eq!(
        run(
            &mut server,
            &mut disk,
            0,
            Packet {
                context: fresh,
                ..packet
            },
            0
        )
        .status,
        Error::Denied as u8
    );
    let new_ref = References::request(workspace2.id, file2.id, fresh).unwrap();
    let found = run(&mut server, &mut disk, 0, new_ref, 0);
    assert_eq!(
        References::decode(&found).unwrap(),
        references(workspace2.id, file2.id)
    );
}

#[test]
fn malformed_ranges_and_versions_are_rejected_without_reading() {
    struct NoIo;
    impl rustic_fs::Disk for NoIo {
        fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), rustic_fs::Error> {
            panic!("rejected request performed disk I/O")
        }
        fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), rustic_fs::Error> {
            panic!("read request wrote to disk")
        }
        fn flush(&mut self) -> Result<(), rustic_fs::Error> {
            panic!("read request flushed disk")
        }
    }
    let (mut server, mut disk, a, _) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let context = grant(&mut server, 0, a, READ_RIGHT, 0);
    let valid = read(references(4, a), 0, 1, 0)
        .packet(READ_OPEN, context)
        .unwrap();
    for (offset, length) in [(0, 0), (0, 1025), (MAX_INTEGER, 1), (u64::MAX, 1)] {
        let mut packet = Packet {
            arg: length,
            ..valid
        };
        packet.data[20..28].copy_from_slice(&offset.to_le_bytes());
        assert_eq!(
            server.handle(&mut NoIo, 0, 10, packet, 0).status,
            Error::Invalid as u8
        );
    }
    let mut past_eof = valid;
    past_eof.data[20..28].copy_from_slice(&1u64.to_le_bytes());
    assert_eq!(
        server.handle(&mut NoIo, 0, 10, past_eof, 0).status,
        Error::Size as u8
    );
    let unpinned = Packet {
        op: READ_CHUNK,
        ..valid
    };
    assert_eq!(
        server.handle(&mut NoIo, 0, 10, unpinned, 0).status,
        Error::Invalid as u8
    );
    let mut unsupported = valid;
    unsupported.data[28..30].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        server.handle(&mut NoIo, 0, 10, unsupported, 0).status,
        Error::UnsupportedVersion as u8
    );
    assert_eq!(server.pending(), 0);
}
