// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_abi::files::{operation::*, reference::*, *};
use rustic_file_service::{Grant, Server};
use sha2::{Digest, Sha256};
use support::{Memory, setup};
fn authorize(server: &mut Server, slot: usize, scope: u32, rights: u8, subject: u64) -> u32 {
    server
        .grant(
            slot,
            Grant {
                peer: slot as u64 + 10,
                endpoint: slot as u64 + 1,
                scope,
                rights,
                generation: 0,
                expires: 0,
                subject,
            },
        )
        .unwrap()
}
fn setup_operations() -> (Server, Memory, Replacement, u32, u32) {
    let (mut server, mut disk, a, b) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    server.volume.enable_operations(&mut disk).unwrap();
    let generation = authorize(&mut server, 0, 0, 7, 9);
    let workspace = Workspace::new([7; 16], 4).unwrap();
    let request = Replacement {
        workspace,
        resource: Resource::new(workspace, a).unwrap(),
        expected_version: Version::new(server.volume.stat(a).unwrap().version).unwrap(),
        retry: Retry {
            epoch: Epoch::new(1).unwrap(),
            key: Key::new(42).unwrap(),
        },
    };
    (server, disk, request, generation, b)
}
fn send(server: &mut Server, disk: &mut Memory, slot: usize, p: Packet) -> Packet {
    server.handle(disk, slot, slot as u64 + 10, p, 1)
}
fn stage(server: &mut Server, disk: &mut Memory, request: Replacement, context: u32, data: &[u8]) {
    assert_eq!(
        send(
            server,
            disk,
            0,
            request.packet(data.len(), context).unwrap()
        )
        .status,
        0
    );
    for (i, bytes) in data.chunks(DATA).enumerate() {
        let mut p = Packet::new(REPLACE_CHUNK);
        p.context = context;
        p.id = request.resource.object();
        p.arg = (i * DATA) as u32;
        p.count = bytes.len() as u8;
        p.data[..bytes.len()].copy_from_slice(bytes);
        assert_eq!(send(server, disk, 0, p).status, 0);
    }
}
fn commit(server: &mut Server, disk: &mut Memory, request: Replacement, context: u32) -> Packet {
    let mut p = Packet::new(REPLACE_COMMIT);
    p.context = context;
    p.id = request.resource.object();
    send(server, disk, 0, p)
}
fn get(server: &mut Server, disk: &mut Memory, lookup: Lookup, context: u32) -> Operation {
    let first = send(server, disk, 0, lookup.packet(context));
    assert_eq!(first.status, 0);
    let mut bytes = [0; RECEIPT_BYTES];
    bytes[..40].copy_from_slice(first.payload());
    for offset in [40, 80] {
        let mut p = Lookup::Id(OperationId::new([7; 16], first.version).unwrap()).packet(context);
        p.op = OPERATION_PART;
        p.arg = offset;
        let r = send(server, disk, 0, p);
        assert_eq!(r.status, 0);
        bytes[offset as usize..offset as usize + r.count as usize].copy_from_slice(r.payload());
    }
    Operation::decode(&bytes).unwrap()
}
#[test]
fn lost_first_reply_recovers_original_receipt_after_edit_delete_restart_and_fresh_grant() {
    let (mut server, mut disk, request, generation, _) = setup_operations();
    stage(
        &mut server,
        &mut disk,
        request,
        generation,
        b"original result",
    );
    let first = commit(&mut server, &mut disk, request, generation);
    assert_eq!(first.status, 0); // Discard its result.
    let lookup = Lookup::Retry {
        workspace: request.workspace,
        retry: request.retry,
    };
    let original = get(&mut server, &mut disk, lookup, generation);
    server
        .volume
        .replace(
            &mut disk,
            request.resource.object(),
            original.version.value(),
            b"later edit",
        )
        .unwrap();
    assert_eq!(
        get(&mut server, &mut disk, Lookup::Id(original.id), generation),
        original
    );
    server
        .volume
        .remove(&mut disk, request.resource.object())
        .unwrap();
    let volume = rustic_fs::Volume::mount(&mut disk).unwrap();
    let mut server = Server::new(volume);
    let unauthorized = send(&mut server, &mut disk, 0, lookup.packet(generation));
    assert_eq!(unauthorized.status, Error::Denied as u8);
    let generation = authorize(&mut server, 0, 4, 7, 9);
    let recovered = get(&mut server, &mut disk, lookup, generation);
    assert_eq!(recovered, original);
    let hash: [u8; 32] = Sha256::digest(b"original result").into();
    assert_eq!(recovered.sha256, hash);
    let sequence = server.volume.sequence();
    stage(
        &mut server,
        &mut disk,
        request,
        generation,
        b"original result",
    );
    assert_eq!(
        commit(&mut server, &mut disk, request, generation).status,
        0
    );
    assert_eq!(server.volume.sequence(), sequence);
    stage(&mut server, &mut disk, request, generation, b"changed");
    assert_eq!(
        commit(&mut server, &mut disk, request, generation).status,
        Error::IdempotencyConflict as u8
    );
}
#[test]
fn operation_fragments_require_current_subject_scope_and_inspection_authority() {
    let (mut server, mut disk, request, generation, b) = setup_operations();
    stage(&mut server, &mut disk, request, generation, b"data");
    let first = commit(&mut server, &mut disk, request, generation);
    let id = OperationId::new([7; 16], first.version).unwrap();
    for (rights, scope, subject, expected) in [
        (1, request.resource.object(), 9, Error::Denied),
        (7, b, 9, Error::OutcomeUnknown),
        (7, 0, 10, Error::OutcomeUnknown),
    ] {
        let context = authorize(&mut server, 1, scope, rights, subject);
        let p = Lookup::Id(id).packet(context);
        assert_eq!(send(&mut server, &mut disk, 1, p).status, expected as u8);
        let missing = Lookup::Id(OperationId::new([7; 16], first.version + 1).unwrap());
        assert_eq!(
            send(&mut server, &mut disk, 1, missing.packet(context)).status,
            expected as u8
        );
    }
    server.revoke(0).unwrap();
    let mut p = Lookup::Id(id).packet(generation);
    p.op = OPERATION_PART;
    p.arg = 40;
    assert_eq!(
        send(&mut server, &mut disk, 0, p).status,
        Error::Revoked as u8
    );
    let context = authorize(&mut server, 0, 0, 7, 9);
    assert_eq!(get(&mut server, &mut disk, Lookup::Id(id), context).id, id);
}
#[test]
fn legacy_packets_cannot_commit_scoped_staging_and_regrant_discards_it() {
    let (mut server, mut disk, request, generation, _) = setup_operations();
    stage(&mut server, &mut disk, request, generation, b"data");
    let mut p = Packet::new(COMMIT);
    p.id = request.resource.object();
    p.context = generation;
    assert_eq!(
        send(&mut server, &mut disk, 0, p).status,
        Error::Protocol as u8
    );
    assert_eq!(server.pending(), 1);
    let context = authorize(&mut server, 0, 0, 7, 9);
    assert_eq!(server.pending(), 0);
    assert_eq!(
        commit(&mut server, &mut disk, request, generation).status,
        Error::Revoked as u8
    );
    assert_eq!(
        commit(&mut server, &mut disk, request, context).status,
        Error::NoTransfer as u8
    );
    assert_eq!(
        server
            .volume
            .stat(request.resource.object())
            .unwrap()
            .version,
        request.expected_version.value()
    );
}
