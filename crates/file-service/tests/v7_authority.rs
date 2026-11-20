// SPDX-License-Identifier: Apache-2.0
//! Owner-issued V7 grants retain the legacy service's scope, rights and root
//! generation rules while all resource checks use native V7 ancestry.
use rustic_abi::files::{
    CANCEL_RIGHT, CREATE, Error, INSPECT_RIGHT, Packet, READ_RIGHT, REMOVE, REPLACE_CHUNK,
    REPLACE_COMMIT, STAT, WRITE_RIGHT,
    operation::{self, OperationId},
    reference::{Epoch, References, Version},
    workspace::{Lookup, Replacement},
};
use rustic_file_service::{Grant7, GrantRequest7, READ_ONLY7, Server7, TRACKED_WRITE7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x8d; 16];
const PEER: u64 = 31;

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

struct Fixture {
    volume: Volume7,
    disk: Sparse,
    alpha: u32,
    alpha_file: u32,
    beta: u32,
    beta_file: u32,
}

fn fixture() -> Fixture {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let alpha = volume
        .create(&mut disk, 4, b"alpha", Kind::Directory)
        .unwrap();
    let alpha_file = volume
        .create(&mut disk, alpha.id, b"inside", Kind::File)
        .unwrap();
    let beta = volume
        .create(&mut disk, 4, b"beta", Kind::Directory)
        .unwrap();
    let beta_file = volume
        .create(&mut disk, beta.id, b"outside", Kind::File)
        .unwrap();
    Fixture {
        volume,
        disk,
        alpha: alpha.id,
        alpha_file: alpha_file.id,
        beta: beta.id,
        beta_file: beta_file.id,
    }
}

fn request(scope: u32, rights: u8, subject: u64, expires: u64, slot: usize) -> GrantRequest7 {
    GrantRequest7 {
        peer: PEER,
        endpoint: 70 + slot as u64,
        scope,
        rights,
        subject,
        expires,
    }
}

fn grant(
    server: &mut Server7<'_>,
    slot: usize,
    scope: u32,
    rights: u8,
    subject: u64,
    expires: u64,
) -> Grant7 {
    server
        .grant(slot, request(scope, rights, subject, expires, slot))
        .unwrap()
}

fn send(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    mut packet: Packet,
    now: u64,
) -> Packet {
    packet.context = context;
    server.handle(disk, slot, PEER, packet, now)
}

fn read_resource(workspace: u32, object: u32, context: u32) -> Packet {
    References::request(workspace, object, context).unwrap()
}

fn tracked_request(
    volume: &Volume7,
    workspace: u32,
    object: u32,
    key: u64,
) -> operation::Replacement {
    let references = References::new(LINEAGE, workspace, object).unwrap();
    operation::Replacement {
        workspace: references.workspace,
        resource: references.resource,
        expected_version: Version::new(volume.stat(object).unwrap().version).unwrap(),
        retry: operation::Retry {
            epoch: Epoch::new(volume.header().unwrap().epoch).unwrap(),
            key: operation::Key::new(key).unwrap(),
        },
    }
}

fn commit_tracked(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    slot: usize,
    context: u32,
    replacement: operation::Replacement,
    bytes: &[u8],
) -> u64 {
    let open = Replacement {
        request: replacement,
    }
    .packet(bytes.len(), context)
    .unwrap();
    assert_eq!(send(server, disk, slot, context, open, 0).status, 0);

    let mut chunk = Packet::new(REPLACE_CHUNK);
    chunk.id = replacement.resource.object();
    chunk.count = bytes.len() as u8;
    chunk.data[..bytes.len()].copy_from_slice(bytes);
    assert_eq!(send(server, disk, slot, context, chunk, 0).status, 0);

    let mut commit = Packet::new(REPLACE_COMMIT);
    commit.id = replacement.resource.object();
    let reply = send(server, disk, slot, context, commit, 0);
    assert_eq!(reply.status, 0);
    reply.version
}

fn operation_lookup(sequence: u64, context: u32) -> Packet {
    Lookup {
        query: operation::Lookup::Id(OperationId::new(LINEAGE, sequence).unwrap()),
    }
    .packet(context)
}

#[test]
fn legacy_right_subsets_and_global_scope_are_preserved_without_inspection_gain() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);

    // Every nonzero subset of the existing four rights remains grantable.
    for rights in 1..=15 {
        let subject = if rights & (INSPECT_RIGHT | CANCEL_RIGHT) == 0 {
            0
        } else {
            41
        };
        let installed = grant(&mut server, 0, 0, rights, subject, 0);
        assert_eq!(
            (installed.scope, installed.rights, installed.subject),
            (0, rights, subject)
        );
    }
    // A READ-only grant may retain a nonzero trusted subject.
    assert_eq!(grant(&mut server, 0, 0, READ_RIGHT, 99, 0).subject, 99);

    for invalid in [
        request(0, 0, 0, 0, 0),
        request(0, 16, 0, 0, 0),
        request(0, INSPECT_RIGHT, 0, 0, 0),
        request(0, CANCEL_RIGHT, 0, 0, 0),
    ] {
        assert_eq!(server.grant(0, invalid), Err(Error::Invalid));
    }

    let global = grant(&mut server, 0, 0, READ_ONLY7, 0, 0);
    for id in [1, 2, 3, 4, f.alpha, f.alpha_file, f.beta, f.beta_file] {
        let stat = send(
            &mut server,
            &mut f.disk,
            0,
            global.context,
            Packet {
                id,
                ..Packet::new(STAT)
            },
            0,
        );
        assert_eq!((stat.status, stat.id), (0, id));
    }

    // A system-root write reaches storage policy and remains read-only.
    let writer = grant(&mut server, 1, 0, WRITE_RIGHT, 0, 0);
    let mut create = Packet::new(CREATE);
    create.id = 1;
    create.count = 4;
    create.data[..4].copy_from_slice(b"file");
    assert_eq!(
        send(&mut server, &mut f.disk, 1, writer.context, create, 0).status,
        Error::ReadOnly as u8
    );

    // READ|WRITE and a subject do not imply INSPECT. The denied lookup must
    // stop before disk I/O and return no identity-bearing fields.
    let limited = grant(&mut server, 2, 0, READ_RIGHT | WRITE_RIGHT, 41, 0);
    let before = f.disk.io_ops;
    let denied = send(
        &mut server,
        &mut f.disk,
        2,
        limited.context,
        operation_lookup(1, limited.context),
        0,
    );
    assert_eq!(denied.status, Error::Denied as u8);
    assert_eq!(
        (denied.id, denied.arg, denied.version, denied.count),
        (0, 0, 0, 0)
    );
    assert_eq!(denied.data, [0; rustic_abi::files::DATA]);
    assert_eq!(f.disk.io_ops, before);
}

#[test]
fn helpers_are_one_level_attenuated_and_inherit_the_roots_subject() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let root = grant(
        &mut server,
        0,
        f.alpha,
        READ_RIGHT | WRITE_RIGHT | INSPECT_RIGHT,
        71,
        100,
    );
    server.extend(0, root.context, f.beta_file).unwrap();

    let helper = server
        .derive(
            1,
            root.context,
            request(f.alpha_file, READ_RIGHT, 999, 50, 1),
            1,
        )
        .unwrap();
    assert_eq!(
        (
            helper.scope,
            helper.second,
            helper.rights,
            helper.expires,
            helper.subject
        ),
        (f.alpha_file, 0, READ_RIGHT, 50, 71)
    );
    assert_eq!(
        server.derive(
            2,
            helper.context,
            request(f.alpha_file, READ_RIGHT, 71, 40, 2),
            2
        ),
        Err(Error::Denied),
        "helpers cannot delegate"
    );
    for invalid in [
        request(f.beta_file, READ_RIGHT, 71, 40, 2),
        request(f.alpha_file, READ_RIGHT | CANCEL_RIGHT, 71, 40, 2),
        request(f.alpha_file, READ_RIGHT, 71, 0, 2),
        request(f.alpha_file, READ_RIGHT, 71, 101, 2),
    ] {
        assert_eq!(
            server.derive(2, root.context, invalid, 2),
            Err(Error::Denied)
        );
    }
    assert_eq!(
        server.derive(
            2,
            root.context,
            request(f.alpha_file, READ_RIGHT, 71, 40, 2),
            100
        ),
        Err(Error::Expired)
    );

    let read = send(
        &mut server,
        &mut f.disk,
        1,
        helper.context,
        read_resource(f.alpha, f.alpha_file, helper.context),
        2,
    );
    assert_eq!(read.status, 0);
    let denied_second = send(
        &mut server,
        &mut f.disk,
        1,
        helper.context,
        read_resource(f.beta, f.beta_file, helper.context),
        2,
    );
    assert_eq!(denied_second.status, Error::Denied as u8);
}

#[test]
fn second_file_scope_is_disjoint_live_and_not_inherited_or_retained_after_removal() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let root = grant(&mut server, 0, f.alpha_file, TRACKED_WRITE7, 88, 0);
    assert_eq!(
        server.extend(0, root.context + 1, f.beta_file),
        Err(Error::Denied)
    );
    assert_eq!(
        server.extend(1, root.context, f.beta_file),
        Err(Error::Denied)
    );
    assert_eq!(
        server.extend(0, root.context, f.beta_file),
        Ok(root.context)
    );
    assert_eq!(
        server.extend(0, root.context, f.beta_file),
        Err(Error::Denied)
    );

    let helper = server
        .derive(
            1,
            root.context,
            request(f.alpha_file, READ_RIGHT, 0, 0, 1),
            0,
        )
        .unwrap();
    assert_eq!(helper.second, 0);

    let allowed = send(
        &mut server,
        &mut f.disk,
        0,
        root.context,
        read_resource(f.beta, f.beta_file, root.context),
        0,
    );
    assert_eq!(allowed.status, 0);
    // The second scope does not let a caller invent a different workspace
    // parent for the resource.
    let before = f.disk.io_ops;
    let wrong_parent = send(
        &mut server,
        &mut f.disk,
        0,
        root.context,
        read_resource(f.alpha, f.beta_file, root.context),
        0,
    );
    assert_eq!(wrong_parent.status, Error::Denied as u8);
    assert_eq!(f.disk.io_ops, before);
    let helper_denied = send(
        &mut server,
        &mut f.disk,
        1,
        helper.context,
        read_resource(f.beta, f.beta_file, helper.context),
        0,
    );
    assert_eq!(helper_denied.status, Error::Denied as u8);

    // Keep a receipt for each independent file. A primary exact scope retains
    // its own removed identity; a second file must still be live to authorize
    // its old receipt.
    let primary_plan = tracked_request(server.volume(), f.alpha, f.alpha_file, 1);
    let primary_sequence = commit_tracked(
        &mut server,
        &mut f.disk,
        0,
        root.context,
        primary_plan,
        b"primary",
    );
    let second_plan = tracked_request(server.volume(), f.beta, f.beta_file, 2);
    let second_sequence = commit_tracked(
        &mut server,
        &mut f.disk,
        0,
        root.context,
        second_plan,
        b"second",
    );

    let mut remove_second = Packet::new(REMOVE);
    remove_second.id = f.beta_file;
    assert_eq!(
        send(&mut server, &mut f.disk, 0, root.context, remove_second, 0).status,
        0
    );
    let removed_second = send(
        &mut server,
        &mut f.disk,
        0,
        root.context,
        read_resource(f.beta, f.beta_file, root.context),
        0,
    );
    assert_eq!(removed_second.status, Error::Denied as u8);
    let mut cached = operation_lookup(second_sequence, root.context);
    cached.op = rustic_abi::files::OPERATION_PART;
    cached.arg = 40;
    assert_eq!(
        send(&mut server, &mut f.disk, 0, root.context, cached, 0).status,
        Error::OutcomeUnknown as u8,
        "cached parts also require the live second file"
    );
    assert_eq!(
        send(
            &mut server,
            &mut f.disk,
            0,
            root.context,
            operation_lookup(second_sequence, root.context),
            0,
        )
        .status,
        Error::OutcomeUnknown as u8
    );

    let mut remove_primary = Packet::new(REMOVE);
    remove_primary.id = f.alpha_file;
    assert_eq!(
        send(&mut server, &mut f.disk, 0, root.context, remove_primary, 0).status,
        0
    );
    assert_eq!(
        send(
            &mut server,
            &mut f.disk,
            0,
            root.context,
            operation_lookup(primary_sequence, root.context),
            0,
        )
        .status,
        0
    );
}

#[test]
fn root_regrant_validates_before_fencing_and_stale_generations_cannot_hit_new_slots() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let root = grant(&mut server, 0, f.alpha, READ_RIGHT | WRITE_RIGHT, 0, 0);
    let helper = server
        .derive(1, root.context, request(f.alpha, READ_RIGHT, 77, 0, 1), 0)
        .unwrap();

    // An invalid replacement leaves both live members untouched.
    assert_eq!(
        server.grant(0, request(u32::MAX, READ_RIGHT, 0, 0, 0)),
        Err(Error::Invalid)
    );
    for (slot, grant) in [(0, root), (1, helper)] {
        let reply = send(
            &mut server,
            &mut f.disk,
            slot,
            grant.context,
            Packet {
                id: f.alpha,
                ..Packet::new(STAT)
            },
            0,
        );
        assert_eq!(reply.status, 0);
    }

    // Replacing the root fences its helper as part of the same returned loss
    // mask. A later independent grant can reuse the helper slot safely.
    let fresh = grant(&mut server, 0, f.alpha, READ_RIGHT, 0, 0);
    assert_eq!(server.grant_at(1).unwrap().rights, 0);
    let fresh_helper = grant(&mut server, 1, f.alpha, READ_RIGHT, 0, 0);
    assert_eq!(server.revoke_root(root.context), 0);
    for (slot, grant) in [(0, fresh), (1, fresh_helper)] {
        let reply = send(
            &mut server,
            &mut f.disk,
            slot,
            grant.context,
            Packet {
                id: f.alpha,
                ..Packet::new(STAT)
            },
            0,
        );
        assert_eq!(reply.status, 0);
    }
    assert_eq!(server.revoke(1), Ok(1 << 1));
    assert_eq!(server.grant_at(0).unwrap().rights, READ_RIGHT);
}

#[test]
fn group_loss_releases_plain_and_staged_candidates_without_touching_disk() {
    for action in 0..5 {
        let mut f = fixture();
        let mut server = Server7::new(&mut f.volume);
        let root = grant(&mut server, 0, 4, TRACKED_WRITE7, 71, 100);
        let helper = server
            .derive(
                1,
                root.context,
                request(f.beta, TRACKED_WRITE7, 0, 90, 1),
                0,
            )
            .unwrap();
        let independent = grant(&mut server, 2, 4, READ_RIGHT, 0, 0);
        let mut plain = Packet::new(rustic_abi::files::BEGIN);
        plain.id = f.alpha_file;
        plain.arg = 1;
        plain.version = server.volume().stat(f.alpha_file).unwrap().version;
        assert_eq!(
            send(&mut server, &mut f.disk, 0, root.context, plain, 0).status,
            0
        );
        let tracked = Replacement {
            request: tracked_request(server.volume(), f.beta, f.beta_file, 1),
        }
        .packet(2000, helper.context)
        .unwrap();
        assert_eq!(
            send(&mut server, &mut f.disk, 1, helper.context, tracked, 0).status,
            0
        );
        assert_eq!(server.pending(), 2);
        assert_eq!(server.volume().open_stages(), 1);
        let before = f.disk.io_ops;
        match action {
            0 => assert_eq!(server.revoke_root(root.context), 3),
            1 => assert_eq!(server.revoke(1), Ok(3)),
            2 => assert_eq!(server.detach(0), 3),
            3 => {
                grant(&mut server, 0, 4, READ_RIGHT, 0, 0);
            }
            4 => assert_eq!(server.expire(100), 3),
            _ => unreachable!(),
        }
        assert_eq!(server.pending(), 0, "action {action}");
        assert_eq!(server.volume().open_stages(), 0);
        assert_eq!(server.grant_at(2), Some(independent));
        assert_eq!(f.disk.io_ops, before);
        // Both bounded slots and the same retry key can be used by fresh authority.
        let fresh = grant(&mut server, 1, 4, TRACKED_WRITE7, 71, 0);
        assert_eq!(
            send(&mut server, &mut f.disk, 1, fresh.context, tracked, 0).status,
            0
        );
        assert_eq!(server.pending(), 1);
    }
}

#[test]
fn detaching_a_helper_preserves_the_root_and_sibling() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let root = grant(&mut server, 0, 4, READ_RIGHT, 0, 0);
    let helper = server
        .derive(1, root.context, request(f.alpha, READ_RIGHT, 0, 0, 1), 0)
        .unwrap();
    let sibling = server
        .derive(2, root.context, request(f.beta, READ_RIGHT, 0, 0, 2), 0)
        .unwrap();
    assert_eq!(server.detach(1), 2);
    assert_eq!(server.grant_at(1), None);
    assert_eq!(server.grant_at(0), Some(root));
    assert_eq!(server.grant_at(2), Some(sibling));
    assert_eq!(server.revoke_root(helper.context), 0);
    assert_eq!(server.revoke_root(root.context), 5);
}

#[test]
fn second_scope_refuses_overlap_directories_global_roots_and_helpers() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let root = grant(&mut server, 0, f.alpha, READ_RIGHT, 0, 0);
    for invalid in [f.alpha, f.alpha_file, f.beta, u32::MAX] {
        assert_eq!(server.extend(0, root.context, invalid), Err(Error::Invalid));
        assert_eq!(server.grant_at(0), Some(root));
    }
    assert_eq!(server.extend(0, root.context, 0), Err(Error::Denied));
    let helper = server
        .derive(
            1,
            root.context,
            request(f.alpha_file, READ_RIGHT, 0, 0, 1),
            0,
        )
        .unwrap();
    assert_eq!(
        server.extend(1, helper.context, f.beta_file),
        Err(Error::Denied)
    );
    let global = grant(&mut server, 2, 0, READ_RIGHT, 0, 0);
    assert_eq!(
        server.extend(2, global.context, f.beta_file),
        Err(Error::Invalid)
    );
    let file = grant(&mut server, 3, f.alpha_file, READ_RIGHT, 0, 0);
    assert_eq!(
        server.extend(3, file.context, f.alpha_file),
        Err(Error::Invalid)
    );
    assert_eq!(
        server.extend(0, root.context, f.beta_file),
        Ok(root.context)
    );
}

#[test]
fn replacing_the_root_forgets_its_helpers_cached_receipt() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let root = grant(&mut server, 0, 4, TRACKED_WRITE7, 71, 0);
    let helper = server
        .derive(
            1,
            root.context,
            request(f.alpha, TRACKED_WRITE7, 0, 0, 1),
            0,
        )
        .unwrap();
    let plan = tracked_request(server.volume(), f.alpha, f.alpha_file, 1);
    let sequence = commit_tracked(
        &mut server,
        &mut f.disk,
        1,
        helper.context,
        plan,
        b"receipt",
    );
    let mut part = operation_lookup(sequence, helper.context);
    part.op = rustic_abi::files::OPERATION_PART;
    part.arg = 40;
    assert_eq!(
        send(&mut server, &mut f.disk, 1, helper.context, part, 0).status,
        0
    );
    grant(&mut server, 0, 4, READ_RIGHT, 0, 0);
    assert_eq!(
        send(&mut server, &mut f.disk, 1, helper.context, part, 0).status,
        Error::Revoked as u8
    );
    let fresh = grant(&mut server, 1, f.alpha, TRACKED_WRITE7, 71, 0);
    assert_eq!(
        send(&mut server, &mut f.disk, 1, fresh.context, part, 0).status,
        Error::OutcomeUnknown as u8
    );
    // Durable history remains recoverable with an explicit fresh lookup.
    assert_eq!(
        send(
            &mut server,
            &mut f.disk,
            1,
            fresh.context,
            operation_lookup(sequence, fresh.context),
            0
        )
        .status,
        0
    );
}
