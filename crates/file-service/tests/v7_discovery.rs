// SPDX-License-Identifier: Apache-2.0
use rustic_abi::{
    files::{
        self, CAPABILITIES, CREATE, Error, Packet, WRITE_RIGHT,
        capabilities::{Bounds, Capabilities},
        negotiation::{self as n, Descriptor},
    },
    services::{Availability, METHODS, Method},
};
use rustic_file_service::{GrantRequest7, Server7};
use rustic_fs::{Disk, Error as FsError, Kind, Volume7};
use std::collections::BTreeMap;

const LINEAGE: [u8; 16] = [0x7c; 16];
const PEER: u64 = 71;
const WORKSPACES: u32 = 4;

#[derive(Default)]
struct Sparse {
    sectors: BTreeMap<u64, [u8; 512]>,
}

impl Disk for Sparse {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        *bytes = self.sectors.get(&sector).copied().unwrap_or([0; 512]);
        Ok(())
    }

    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
        self.sectors.insert(sector, *bytes);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), FsError> {
        Ok(())
    }
}

struct NoIo;

impl Disk for NoIo {
    fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), FsError> {
        panic!("discovery read disk")
    }

    fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), FsError> {
        panic!("discovery wrote disk")
    }

    fn flush(&mut self) -> Result<(), FsError> {
        panic!("discovery flushed disk")
    }
}

struct FailingWrites<'a>(&'a mut Sparse);

impl Disk for FailingWrites<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        self.0.read(sector, bytes)
    }

    fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), FsError> {
        Err(FsError::Io)
    }

    fn flush(&mut self) -> Result<(), FsError> {
        Err(FsError::Io)
    }
}

struct Fixture {
    volume: Volume7,
    disk: Sparse,
}

fn fixture() -> Fixture {
    let mut disk = Sparse::default();
    let mut volume = Volume7::EMPTY;
    volume.provision_into(&mut disk, LINEAGE).unwrap();
    volume
        .create(&mut disk, WORKSPACES, b"alpha", Kind::Directory)
        .unwrap();
    Fixture { volume, disk }
}

fn grant(server: &mut Server7<'_>, expires: u64) -> u32 {
    server
        .grant(
            0,
            GrantRequest7 {
                peer: PEER,
                endpoint: 90,
                scope: WORKSPACES,
                // A write-only grant proves that discovery requires no specific
                // operation right.
                rights: WRITE_RIGHT,
                subject: 0,
                expires,
            },
        )
        .unwrap()
        .context
}

fn capabilities_request(context: u32) -> Packet {
    Packet {
        context,
        ..Packet::new(CAPABILITIES)
    }
}

fn assert_error(packet: Packet, op: u8, context: u32, error: Error) {
    assert_eq!(
        (packet.op, packet.context, packet.status),
        (op, context, error as u8)
    );
    assert_eq!(
        (packet.id, packet.arg, packet.version, packet.count),
        (0, 0, 0, 0)
    );
    assert_eq!(packet.data, [0; files::DATA]);
}

#[test]
fn capabilities_are_the_exact_mounted_v7_catalog_and_bounds() {
    let mut fixture = fixture();
    let mut server = Server7::new(&mut fixture.volume);
    let context = grant(&mut server, 0);
    let reply = server.handle(&mut NoIo, 0, PEER, capabilities_request(context), 0);

    let expected = Capabilities {
        availability: [
            Availability::Degraded,
            Availability::Unavailable,
            Availability::Available,
            Availability::Available,
            Availability::Available,
            Availability::Unavailable,
            Availability::Unavailable,
            Availability::Unavailable,
        ],
        bounds: Bounds {
            max_inline_bytes: 1024,
            max_page_items: METHODS as u8,
            receipt_capacity: 8,
        },
    };
    assert_eq!(Capabilities::decode(&reply), Ok(expected));
    assert_eq!(reply, expected.packet(context).unwrap());
    assert_eq!(reply.context, context);
    assert_eq!(expected.of(Method::FilesRead), Availability::Available);
    assert_eq!(expected.of(Method::FilesReplace), Availability::Available);
    assert_eq!(expected.of(Method::OperationsGet), Availability::Available);
    assert_eq!(
        expected.of(Method::OperationsCancel),
        Availability::Unavailable
    );
}

#[test]
fn capabilities_require_a_live_peer_context_and_unexpired_grant_only() {
    let mut fixture = fixture();
    let mut server = Server7::new(&mut fixture.volume);
    assert_error(
        server.handle(&mut NoIo, 0, PEER, capabilities_request(0), 4),
        CAPABILITIES,
        0,
        Error::Denied,
    );
    let context = grant(&mut server, 5);
    let request = capabilities_request(context);

    assert_eq!(
        server.handle(&mut NoIo, 0, PEER, request, 4).status,
        0,
        "write-only authority may query implementation availability"
    );
    assert_error(
        server.handle(&mut NoIo, 0, PEER + 1, request, 4),
        CAPABILITIES,
        context,
        Error::Denied,
    );
    assert_error(
        server.handle(&mut NoIo, 0, PEER, capabilities_request(context + 1), 4),
        CAPABILITIES,
        context + 1,
        Error::Revoked,
    );
    assert_error(
        server.handle(&mut NoIo, 0, PEER, request, 5),
        CAPABILITIES,
        context,
        Error::Expired,
    );
    server.revoke(0).unwrap();
    assert_error(
        server.handle(&mut NoIo, 0, PEER, request, 4),
        CAPABILITIES,
        context,
        Error::Revoked,
    );
}

#[test]
fn discovery_rejects_malformed_frames_and_refuses_unimplemented_descriptors() {
    let mut fixture = fixture();
    let mut server = Server7::new(&mut fixture.volume);
    let context = grant(&mut server, 0);

    for mutate in [
        |p: &mut Packet| p.id = 1,
        |p: &mut Packet| p.arg = 1,
        |p: &mut Packet| p.version = 1,
        |p: &mut Packet| p.count = 1,
    ] {
        let mut malformed = capabilities_request(context);
        mutate(&mut malformed);
        assert_error(
            server.handle(&mut NoIo, 0, PEER, malformed, 0),
            CAPABILITIES,
            context,
            Error::Protocol,
        );
    }
    let mut nonzero_tail = capabilities_request(context);
    nonzero_tail.data[0] = 1;
    assert_error(
        server.handle(&mut NoIo, 0, PEER, nonzero_tail, 0),
        CAPABILITIES,
        context,
        Error::Protocol,
    );

    for method in [Method::OperationsGet, Method::OperationsCancel] {
        let request = n::request(method, context).unwrap();
        assert_error(
            server.handle(&mut NoIo, 0, PEER, request, 0),
            n::DESCRIBE,
            context,
            Error::Unavailable,
        );
        assert_eq!(n::decode_request(&request), Ok(method));
    }

    let good = n::request(Method::OperationsGet, context).unwrap();
    type MalformedCase = (fn(&mut Packet), Error);
    let malformed_cases: [MalformedCase; 5] = [
        (|p: &mut Packet| p.count = 1, Error::Protocol),
        (|p: &mut Packet| p.data[0] = 1, Error::Protocol),
        (|p: &mut Packet| p.version += 1, Error::UnsupportedVersion),
        (|p: &mut Packet| p.arg += 1, Error::UnsupportedVersion),
        (|p: &mut Packet| p.id = 4, Error::Unsupported),
    ];
    for (mutate, expected) in malformed_cases {
        let mut malformed = good;
        mutate(&mut malformed);
        assert_error(
            server.handle(&mut NoIo, 0, PEER, malformed, 0),
            n::DESCRIBE,
            context,
            expected,
        );
    }
    // V7 refuses DESCRIBE without manufacturing a queue size to satisfy the
    // reviewed descriptor codec's nonzero execution-ticket rule.
    assert_eq!(
        Descriptor::reviewed(
            Method::OperationsGet,
            Availability::Unavailable,
            n::Limits {
                retained_operations: 8,
                execution_tickets: 0,
                active_publications: 1,
            },
        )
        .unwrap()
        .packet(context),
        Err(Error::Protocol)
    );
}

#[test]
fn fenced_mount_refuses_discovery_without_disk_access() {
    let mut fixture = fixture();
    let mut server = Server7::new(&mut fixture.volume);
    let context = grant(&mut server, 0);
    let mut create = Packet::new(CREATE);
    create.context = context;
    create.id = WORKSPACES;
    create.count = 6;
    create.data[..6].copy_from_slice(b"broken");
    let failed = server.handle(&mut FailingWrites(&mut fixture.disk), 0, PEER, create, 0);
    assert_eq!(failed.status, Error::Uncertain as u8);
    assert_eq!(server.volume().header(), Err(FsError::Uncertain));

    assert_error(
        server.handle(&mut NoIo, 0, PEER, capabilities_request(context), 0),
        CAPABILITIES,
        context,
        Error::Uncertain,
    );
    let describe = n::request(Method::OperationsGet, context).unwrap();
    assert_error(
        server.handle(&mut NoIo, 0, PEER, describe, 0),
        n::DESCRIBE,
        context,
        Error::Uncertain,
    );
}
