// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, LIST, LOOKUP, Packet, READ, STAT, WRITE_RIGHT};
use rustic_file_service::{ADMISSION7, Grant7, GrantRequest7, READ_ONLY7, Server7, TRACKED_WRITE7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x71; 16];
const WORKSPACES: u32 = 4;

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
    workspace: u32,
    directory: u32,
    file: u32,
    sibling: u32,
}

fn setup() -> Fixture {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let directory = volume
        .create(&mut disk, WORKSPACES, b"alpha", Kind::Directory)
        .unwrap();
    let file = volume
        .create(&mut disk, directory.id, b"note", Kind::File)
        .unwrap();
    let sibling = volume
        .create(&mut disk, directory.id, b"peer", Kind::File)
        .unwrap();
    volume
        .replace(&mut disk, file.id, file.version, b"private file bytes")
        .unwrap();
    Fixture {
        volume,
        disk,
        workspace: WORKSPACES,
        directory: directory.id,
        file: file.id,
        sibling: sibling.id,
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

fn grant(server: &mut Server7<'_>, slot: usize, scope: u32, peer: u64) -> Grant7 {
    server
        .grant(slot, read_only(peer, 80 + slot as u64, scope, 0))
        .unwrap()
}

fn request(op: u8, id: u32, context: u32) -> Packet {
    Packet {
        id,
        context,
        ..Packet::new(op)
    }
}

fn lookup(parent: u32, name: &[u8], context: u32) -> Packet {
    let mut packet = request(LOOKUP, parent, context);
    packet.count = name.len() as u8;
    packet.data[..name.len()].copy_from_slice(name);
    packet
}

fn node_name(packet: &Packet) -> &[u8] {
    &packet.data[8..8 + usize::from(packet.data[2])]
}

fn assert_zero_error(packet: &Packet, error: Error, context: u32) {
    assert_eq!(packet.status, error as u8);
    assert_eq!(packet.context, context);
    assert_eq!(
        (packet.id, packet.arg, packet.version, packet.count),
        (0, 0, 0, 0)
    );
    assert_eq!(packet.data, [0; rustic_abi::files::DATA]);
}

#[test]
fn scope_zero_reaches_the_mounted_volume_and_virtual_root_lists_all_four_roots() {
    let mut fixture = setup();
    let mut server = Server7::new(&mut fixture.volume);
    let authorization = grant(&mut server, 0, 0, 11);
    assert_eq!(authorization.scope, 0);

    for (id, name) in [
        (1, b"system".as_slice()),
        (2, b"data"),
        (3, b"config"),
        (4, b"workspaces"),
    ] {
        let found = server.handle(
            &mut fixture.disk,
            0,
            11,
            lookup(0, name, authorization.context),
            0,
        );
        assert_eq!(found.status, 0);
        assert_eq!(
            (found.id, found.arg, found.version, found.count),
            (id, 0, 1, 40)
        );
        assert_eq!(found.data[0], Kind::Directory as u8);
        assert_eq!(node_name(&found), name);
        assert_eq!(found.data[18..], [0; 22]);
    }
    let found = server.handle(
        &mut fixture.disk,
        0,
        11,
        lookup(0, b"workspaces", authorization.context),
        0,
    );
    assert_eq!(u32::from_le_bytes(found.data[4..8].try_into().unwrap()), 0);

    let mut root_list = request(LIST, 0, authorization.context);
    for (cursor, (id, name)) in [
        (1, b"system".as_slice()),
        (2, b"data"),
        (3, b"config"),
        (4, b"workspaces"),
    ]
    .into_iter()
    .enumerate()
    {
        root_list.arg = cursor as u32;
        let listed = server.handle(&mut fixture.disk, 0, 11, root_list, 0);
        assert_eq!(
            (listed.status, listed.id, listed.count, listed.data[3]),
            (0, id, 40, cursor as u8 + 1)
        );
        assert_eq!(node_name(&listed), name);
    }
    root_list.arg = 4;
    let end = server.handle(&mut fixture.disk, 0, 11, root_list, 0);
    assert_eq!(end.status, 0);
    assert_eq!((end.id, end.arg, end.version, end.count), (0, 0, 0, 0));
    assert_eq!(end.data, [0; rustic_abi::files::DATA]);

    assert_zero_error(
        &server.handle(
            &mut fixture.disk,
            0,
            11,
            lookup(0, b"missing", authorization.context),
            0,
        ),
        Error::NotFound,
        authorization.context,
    );
    assert_zero_error(
        &server.handle(
            &mut fixture.disk,
            0,
            11,
            request(STAT, 0, authorization.context),
            0,
        ),
        Error::Denied,
        authorization.context,
    );
}

#[test]
fn workspace_and_narrow_scopes_keep_virtual_roots_filtered() {
    let mut fixture = setup();
    let mut server = Server7::new(&mut fixture.volume);
    let workspace = grant(&mut server, 0, WORKSPACES, 11);
    let directory = grant(&mut server, 1, fixture.directory, 12);
    let file = grant(&mut server, 2, fixture.file, 13);

    let root = server.handle(
        &mut fixture.disk,
        0,
        11,
        lookup(0, b"workspaces", workspace.context),
        0,
    );
    assert_eq!((root.status, root.id), (0, WORKSPACES));
    for hidden in [b"system".as_slice(), b"data", b"config", b"missing"] {
        assert_zero_error(
            &server.handle(
                &mut fixture.disk,
                0,
                11,
                lookup(0, hidden, workspace.context),
                0,
            ),
            Error::Denied,
            workspace.context,
        );
    }
    let mut list = request(LIST, 0, workspace.context);
    let only = server.handle(&mut fixture.disk, 0, 11, list, 0);
    assert_eq!((only.status, only.id, only.data[3]), (0, WORKSPACES, 1));
    list.arg = 1;
    assert_eq!(server.handle(&mut fixture.disk, 0, 11, list, 0).id, 0);

    for (slot, peer, grant) in [(1, 12, directory), (2, 13, file)] {
        assert_zero_error(
            &server.handle(
                &mut fixture.disk,
                slot,
                peer,
                lookup(0, b"workspaces", grant.context),
                0,
            ),
            Error::Denied,
            grant.context,
        );
        assert_zero_error(
            &server.handle(
                &mut fixture.disk,
                slot,
                peer,
                request(LIST, 0, grant.context),
                0,
            ),
            Error::Denied,
            grant.context,
        );
    }
}

#[test]
fn scopes_reach_their_own_node_and_descendants_but_hide_siblings_and_ancestors() {
    let mut fixture = setup();
    let mut server = Server7::new(&mut fixture.volume);
    let authorization = grant(&mut server, 0, fixture.file, 11);

    let own = server.handle(
        &mut fixture.disk,
        0,
        11,
        request(STAT, fixture.file, authorization.context),
        0,
    );
    assert_eq!(
        (own.status, own.id, own.arg, own.count),
        (0, fixture.file, 18, 40)
    );
    assert_eq!(node_name(&own), b"note");

    let own_read = server.handle(
        &mut fixture.disk,
        0,
        11,
        request(READ, fixture.file, authorization.context),
        0,
    );
    assert_eq!(
        (own_read.status, own_read.id, own_read.arg, own_read.count),
        (0, fixture.file, 18, 18)
    );
    assert_eq!(&own_read.data[..18], b"private file bytes");

    for packet in [
        request(STAT, fixture.directory, authorization.context),
        request(STAT, fixture.sibling, authorization.context),
        lookup(fixture.directory, b"peer", authorization.context),
        request(LIST, fixture.directory, authorization.context),
        request(READ, fixture.sibling, authorization.context),
    ] {
        assert_zero_error(
            &server.handle(&mut fixture.disk, 0, 11, packet, 0),
            Error::Denied,
            authorization.context,
        );
    }

    assert_zero_error(
        &server.handle(
            &mut fixture.disk,
            0,
            11,
            request(LIST, 0, authorization.context),
            0,
        ),
        Error::Denied,
        authorization.context,
    );
}

#[test]
fn rights_subsets_install_but_a_write_only_grant_cannot_query() {
    let mut fixture = setup();
    let mut server = Server7::new(&mut fixture.volume);
    for (slot, rights) in [READ_ONLY7, TRACKED_WRITE7, ADMISSION7]
        .into_iter()
        .enumerate()
    {
        let grant = server
            .grant(
                slot,
                GrantRequest7 {
                    peer: 11 + slot as u64,
                    endpoint: 80 + slot as u64,
                    scope: fixture.workspace,
                    rights,
                    subject: if rights == READ_ONLY7 {
                        0
                    } else {
                        100 + slot as u64
                    },
                    expires: 0,
                },
            )
            .unwrap();
        let response = server.handle(
            &mut fixture.disk,
            slot,
            grant.peer,
            request(STAT, fixture.file, grant.context),
            0,
        );
        assert_eq!((response.status, response.id), (0, fixture.file));
    }

    let write_only = server
        .grant(
            3,
            GrantRequest7 {
                peer: 50,
                endpoint: 90,
                scope: fixture.workspace,
                rights: WRITE_RIGHT,
                subject: 7,
                expires: 0,
            },
        )
        .unwrap();
    let denied = server.handle(
        &mut fixture.disk,
        3,
        50,
        request(STAT, fixture.file, write_only.context),
        0,
    );
    assert_eq!(denied.status, Error::Denied as u8);

    assert_eq!(
        server.grant(
            3,
            GrantRequest7 {
                peer: 50,
                endpoint: 90,
                scope: fixture.workspace,
                rights: 0,
                subject: 7,
                expires: 0,
            }
        ),
        Err(Error::Invalid)
    );
}

#[test]
fn malformed_namespace_packets_have_zero_replies_and_do_not_touch_the_disk() {
    let mut fixture = setup();
    let mut server = Server7::new(&mut fixture.volume);
    let authorization = grant(&mut server, 0, fixture.workspace, 11);
    let io_before = fixture.disk.io_ops;

    let mut malformed_lookup = lookup(fixture.workspace, b"alpha", authorization.context);
    malformed_lookup.data[31] = 1;
    let mut oversized_cursor = request(LIST, fixture.workspace, authorization.context);
    oversized_cursor.arg = 256;
    let mut malformed_stat = request(STAT, fixture.file, authorization.context);
    malformed_stat.count = 1;
    malformed_stat.data[0] = 1;
    let mut malformed_read = request(READ, fixture.file, authorization.context);
    malformed_read.count = 1;

    for (packet, error) in [
        (malformed_lookup, Error::Protocol),
        (oversized_cursor, Error::Invalid),
        (malformed_stat, Error::Protocol),
        (malformed_read, Error::Protocol),
    ] {
        assert_zero_error(
            &server.handle(&mut fixture.disk, 0, 11, packet, 0),
            error,
            authorization.context,
        );
    }
    assert_eq!(fixture.disk.io_ops, io_before);
}

#[test]
fn list_cursor_is_an_authorized_child_ordinal_even_when_slot_256_is_occupied() {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    let mut final_node = 0;
    for index in 0..252 {
        let name = format!("item{index:03}");
        let node = volume
            .create(&mut disk, WORKSPACES, name.as_bytes(), Kind::File)
            .unwrap();
        final_node = node.id;
    }
    let (physical_next, physical_final) = volume.list(WORKSPACES, 255).unwrap().unwrap();
    assert_eq!(physical_next, 256);
    assert_eq!(physical_final.id, final_node);

    let mut server = Server7::new(&mut volume);
    let authorization = grant(&mut server, 0, WORKSPACES, 11);
    let mut last = request(LIST, WORKSPACES, authorization.context);
    last.arg = 251;
    let last = server.handle(&mut disk, 0, 11, last, 0);
    assert_eq!((last.status, last.id, last.data[3]), (0, final_node, 252));
    assert_eq!(node_name(&last), b"item251");

    let mut end = request(LIST, WORKSPACES, authorization.context);
    end.arg = u32::from(last.data[3]);
    let end = server.handle(&mut disk, 0, 11, end, 0);
    assert_eq!(
        (end.status, end.id, end.arg, end.version, end.count),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(end.data, [0; rustic_abi::files::DATA]);
}
