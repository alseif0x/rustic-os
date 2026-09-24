// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{
    BEGIN, CREATE, Error, OPERATION_ID, Packet, READ_CHUNK, READ_OPEN, REMOVE, REPLACE_OPEN,
    admission,
    read::{Header, Request},
    reference::{References, Version},
};
use rustic_file_service::{CLIENTS7, Grant7, GrantRequest7, READ_ONLY7, Server7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7, WriteIdentity7};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x7a; 16];

#[derive(Default)]
struct Sparse {
    sectors: BTreeMap<u64, [u8; 512]>,
    io_ops: usize,
}

impl Disk for Sparse {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        self.io_ops += 1;
        *bytes = self.sectors.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
        self.io_ops += 1;
        self.sectors.insert(sector, *bytes);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), FsError> {
        self.io_ops += 1;
        Ok(())
    }
}

fn setup(bytes: &[u8]) -> (Volume7, Sparse, u32, u32, u32) {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let workspace = volume
        .create(&mut disk, 4, b"alpha", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, workspace.id, b"note", Kind::File)
        .unwrap();
    let sibling = volume
        .create(&mut disk, workspace.id, b"other", Kind::File)
        .unwrap();
    let identity = WriteIdentity7 {
        subject: 9,
        workspace: workspace.id,
        object: file.id,
        instance: 1,
        retry_epoch: volume.header().unwrap().epoch,
        retry_key: 1,
    };
    volume
        .replace_tracked(&mut disk, identity, file.version, bytes)
        .unwrap();
    (volume, disk, workspace.id, file.id, sibling.id)
}

fn read_request(
    workspace: u32,
    object: u32,
    lineage: [u8; 16],
    offset: u64,
    length: u16,
    version: Option<u64>,
) -> Request {
    let references = References::new(lineage, workspace, object).unwrap();
    Request {
        workspace: references.workspace,
        resource: references.resource,
        expected_version: version.map(|value| Version::new(value).unwrap()),
        offset,
        length,
    }
}

fn read_only(peer: u64, endpoint: u64, scope: u32, expires: u64) -> GrantRequest7 {
    GrantRequest7 {
        peer,
        endpoint,
        scope,
        rights: READ_ONLY7,
        subject: 0,
        expires,
    }
}

fn grant(server: &mut Server7<'_>, slot: usize, scope: u32, peer: u64, expiry: u64) -> Grant7 {
    server
        .grant(slot, read_only(peer, 90 + slot as u64, scope, expiry))
        .unwrap()
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[test]
fn references_open_and_chunk_use_the_mounted_v7_identity_and_exact_range_bytes() {
    let content = b"bounded V7 read";
    let (mut volume, mut disk, workspace, file, _) = setup(content);
    let mut server = Server7::new(&mut volume);
    let authorization = grant(&mut server, 0, workspace, 11, 0);
    assert_eq!(authorization.endpoint, 90);
    assert_eq!(server.grant_at(0), Some(authorization));

    let reference_reply = server.handle(
        &mut disk,
        0,
        11,
        References::request(workspace, file, authorization.context).unwrap(),
        0,
    );
    let references = References::decode(&reference_reply).unwrap();
    assert_eq!(references.workspace.lineage(), LINEAGE);
    assert_eq!(references.workspace.root(), workspace);
    assert_eq!(references.resource.object(), file);

    let open = read_request(workspace, file, LINEAGE, 3, 7, None)
        .packet(READ_OPEN, authorization.context)
        .unwrap();
    let opened = server.handle(&mut disk, 0, 11, open, 0);
    let header = Header::decode(&opened).unwrap();
    assert_eq!(header.id, file);
    assert_eq!(header.size, content.len() as u64);
    assert_eq!(header.range_sha256, digest(&content[3..10]));
    assert_eq!(header.retry_epoch.value(), 1);

    let chunk = read_request(workspace, file, LINEAGE, 3, 7, Some(header.version.value()))
        .packet(READ_CHUNK, authorization.context)
        .unwrap();
    let result = server.handle(&mut disk, 0, 11, chunk, 0);
    assert_eq!(result.status, 0);
    assert_eq!(
        (result.id, result.arg, result.version, result.count),
        (file, content.len() as u32, header.version.value(), 7)
    );
    assert_eq!(&result.data[..7], &content[3..10]);
    assert_eq!(&result.data[7..], &[0; 33]);
}

#[test]
fn verified_ancestry_scope_and_lineage_are_all_required() {
    let (mut volume, mut disk, workspace, file, sibling) = setup(b"inside");
    let mut server = Server7::new(&mut volume);
    let directory_grant = grant(&mut server, 0, workspace, 11, 0);
    assert_eq!(
        server.grant(3, read_only(33, 93, 1, 0)),
        Err(Error::Denied),
        "a grant cannot widen outside the workspaces tree"
    );

    for request in [
        read_request(workspace, file, [0x33; 16], 0, 1, None),
        read_request(2, file, LINEAGE, 0, 1, None),
    ] {
        let response = server.handle(
            &mut disk,
            0,
            11,
            request.packet(READ_OPEN, directory_grant.context).unwrap(),
            0,
        );
        assert_eq!(response.status, Error::Denied as u8);
        assert_eq!(
            (response.id, response.arg, response.version, response.count),
            (0, 0, 0, 0)
        );
    }

    let mut file_scoped = Server7::new(&mut volume);
    let scoped_grant = grant(&mut file_scoped, 0, file, 11, 0);
    let denied_sibling = read_request(workspace, sibling, LINEAGE, 0, 1, None)
        .packet(READ_OPEN, scoped_grant.context)
        .unwrap();
    assert_eq!(
        file_scoped
            .handle(&mut disk, 0, 11, denied_sibling, 0)
            .status,
        Error::Denied as u8
    );
}

#[test]
fn slots_bind_peer_context_expiry_and_endpoint_lifecycle() {
    let (mut volume, mut disk, workspace, file, _) = setup(b"slot");
    let mut server = Server7::new(&mut volume);
    let first = grant(&mut server, 0, workspace, 11, 10);
    let second = grant(&mut server, 1, file, 22, 0);
    assert_ne!(first.context, second.context);
    assert_eq!(server.grant_at(1), Some(second));

    let old_packet = read_request(workspace, file, LINEAGE, 0, 1, None)
        .packet(READ_OPEN, first.context)
        .unwrap();
    assert_eq!(
        server.handle(&mut disk, 0, 99, old_packet, 0).status,
        Error::Denied as u8
    );
    assert_eq!(
        server.handle(&mut disk, 0, 11, old_packet, 10).status,
        Error::Expired as u8
    );
    assert_eq!(server.expire(9), 0);
    assert_eq!(server.expire(10), 1 << 0);
    assert_eq!(server.expire(11), 0);
    assert_eq!(server.grant_at(0), Some(first));
    assert_eq!(
        server.handle(&mut disk, 0, 11, old_packet, 10).status,
        Error::Revoked as u8
    );

    let replacement = grant(&mut server, 0, workspace, 11, 0);
    assert_ne!(replacement.context, first.context);
    assert_eq!(
        server.handle(&mut disk, 0, 11, old_packet, 0).status,
        Error::Revoked as u8
    );
    server.revoke(0).unwrap();
    assert_eq!(server.grant_at(0), Some(replacement));
    assert_eq!(
        server
            .handle(
                &mut disk,
                0,
                11,
                read_request(workspace, file, LINEAGE, 0, 1, None)
                    .packet(READ_OPEN, replacement.context)
                    .unwrap(),
                0,
            )
            .status,
        Error::Revoked as u8
    );
    server.detach(0);
    assert_eq!(server.grant_at(0), None);
    assert_eq!(
        server.handle(&mut disk, 0, 11, old_packet, 0).status,
        Error::Denied as u8
    );
}

#[test]
fn eof_version_and_request_validation_keep_the_existing_read_contract() {
    let content = b"abc";
    let (mut volume, mut disk, workspace, file, _) = setup(content);
    let mut server = Server7::new(&mut volume);
    let authorization = grant(&mut server, 0, workspace, 11, 0);

    let eof = read_request(workspace, file, LINEAGE, 3, 1, None)
        .packet(READ_OPEN, authorization.context)
        .unwrap();
    let header = Header::decode(&server.handle(&mut disk, 0, 11, eof, 0)).unwrap();
    assert_eq!(header.size, 3);
    assert_eq!(header.range_sha256, digest(&[]));

    let beyond_eof = read_request(workspace, file, LINEAGE, 4, 1, None)
        .packet(READ_OPEN, authorization.context)
        .unwrap();
    assert_eq!(
        server.handle(&mut disk, 0, 11, beyond_eof, 0).status,
        Error::Size as u8
    );

    let stale_version = read_request(workspace, file, LINEAGE, 0, 1, Some(1))
        .packet(READ_CHUNK, authorization.context)
        .unwrap();
    assert_eq!(
        server.handle(&mut disk, 0, 11, stale_version, 0).status,
        Error::Version as u8
    );

    let malformed = Packet {
        count: 29,
        ..read_request(workspace, file, LINEAGE, 0, 1, None)
            .packet(READ_OPEN, authorization.context)
            .unwrap()
    };
    assert_eq!(
        server.handle(&mut disk, 0, 11, malformed, 0).status,
        Error::Protocol as u8
    );
}

#[test]
fn maximum_profile2_file_size_is_reported_while_range_stays_bounded() {
    let bytes: Vec<u8> = (0..rustic_abi::files::workspace::MAX_FILE_BYTES)
        .map(|index| index as u8)
        .collect();
    let (mut volume, mut disk, workspace, file, _) = setup(&bytes);
    let mut server = Server7::new(&mut volume);
    let authorization = grant(&mut server, 0, workspace, 11, 0);
    let offset = bytes.len() as u64 - 1;
    let open = read_request(workspace, file, LINEAGE, offset, 1, None)
        .packet(READ_OPEN, authorization.context)
        .unwrap();
    let header = Header::decode(&server.handle(&mut disk, 0, 11, open, 0)).unwrap();
    assert_eq!(header.size, bytes.len() as u64);
    assert_eq!(header.range_sha256, digest(&bytes[bytes.len() - 1..]));

    let chunk = read_request(
        workspace,
        file,
        LINEAGE,
        offset,
        1,
        Some(header.version.value()),
    )
    .packet(READ_CHUNK, authorization.context)
    .unwrap();
    let result = server.handle(&mut disk, 0, 11, chunk, 0);
    assert_eq!((result.arg, result.count), (bytes.len() as u32, 1));
    assert_eq!(result.data[0], bytes[bytes.len() - 1]);
}

#[test]
fn mutations_admissions_and_operation_queries_are_not_dispatched() {
    let (mut volume, mut disk, workspace, _, _) = setup(b"read only");
    let mut server = Server7::new(&mut volume);
    let authorization = grant(&mut server, 0, workspace, 11, 0);
    let io_before = disk.io_ops;

    let mut create = Packet::new(CREATE);
    create.count = 1;
    create.data[0] = b'x';
    let mut replace = Packet::new(REPLACE_OPEN);
    replace.count = 36;
    let mut operation_id = Packet::new(OPERATION_ID);
    operation_id.count = 16;
    let mut admission_open = Packet::new(admission::OPEN);
    admission_open.count = 36;
    let mut admission_observe = Packet::new(admission::OBSERVE);
    admission_observe.arg = admission::OBSERVATION_VERSION;
    admission_observe.count = 16;

    for packet in [
        create,
        Packet::new(REMOVE),
        Packet::new(BEGIN),
        replace,
        operation_id,
        admission_open,
        admission_observe,
    ] {
        let packet = Packet {
            context: authorization.context,
            ..packet
        };
        let response = server.handle(&mut disk, 0, 11, packet, 0);
        assert_eq!(
            response.status,
            Error::Unsupported as u8,
            "op {}",
            packet.op
        );
        assert_eq!(
            (response.id, response.arg, response.version, response.count),
            (0, 0, 0, 0)
        );
        assert_eq!(response.data, [0; rustic_abi::files::DATA]);
    }
    assert_eq!(
        server
            .handle(
                &mut disk,
                0,
                99,
                Packet {
                    context: authorization.context,
                    ..create
                },
                0
            )
            .status,
        Error::Denied as u8
    );
    assert_eq!(CLIENTS7, 4);
    assert_eq!(disk.io_ops, io_before);
}
