// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_abi::files::{CAPABILITIES, Error, Packet, READ_RIGHT, capabilities::Capabilities};
use rustic_abi::services::{Availability, METHODS, Method};
use support::{grant, run, setup};

fn ask(context: u32) -> Packet {
    Packet {
        context,
        ..Packet::new(CAPABILITIES)
    }
}

fn report(
    server: &mut rustic_file_service::Server,
    disk: &mut support::Memory,
    context: u32,
) -> Capabilities {
    let reply = run(server, disk, 0, ask(context), 1);
    assert_eq!(reply.status, 0);
    Capabilities::decode(&reply).unwrap()
}

#[test]
fn discovery_reports_the_mounted_volume_rather_than_a_build_time_promise() {
    let (mut s, mut d, _, _) = setup();
    let context = grant(&mut s, 0, 0, READ_RIGHT, 0);
    let before = report(&mut s, &mut d, context);
    for (method, expected) in [
        (Method::CapabilitiesList, Availability::Degraded),
        (Method::CapabilitiesDescribe, Availability::Unavailable),
        (Method::FilesRead, Availability::Available),
        (Method::FilesReplace, Availability::Unavailable),
        (Method::OperationsGet, Availability::Unavailable),
        (Method::OperationsCancel, Availability::Unavailable),
        (Method::EventsRead, Availability::Unavailable),
        (Method::SystemStatus, Availability::Unavailable),
    ] {
        assert_eq!(before.of(method), expected, "{}", method.name());
    }
    s.volume.enable_recovery(&mut d, [7; 16]).unwrap();
    s.volume.enable_operations(&mut d).unwrap();
    let after = report(&mut s, &mut d, context);
    assert_eq!(after.of(Method::FilesReplace), Availability::Available);
    assert_eq!(after.of(Method::OperationsGet), Availability::Available);
    // Methods owned by other services stay unavailable instead of disappearing.
    assert_eq!(after.of(Method::EventsRead), Availability::Unavailable);
    assert_eq!(after.bounds.max_inline_bytes, 1024);
    assert_eq!(after.bounds.receipt_capacity, 2);
    assert_eq!(after.bounds.max_page_items as usize, METHODS);
}

#[test]
fn discovery_needs_a_live_grant_but_no_particular_right_and_leaks_nothing() {
    let (mut s, mut d, file, _) = setup();
    // No grant at all: the service refuses before reporting anything.
    assert_eq!(
        run(&mut s, &mut d, 0, ask(0), 1).status,
        Error::Denied as u8
    );
    let context = grant(&mut s, 0, 0, READ_RIGHT, 0);
    let reply = run(&mut s, &mut d, 0, ask(context), 1);
    assert_eq!(reply.status, 0);
    assert_eq!((reply.id, reply.context), (0, context));
    // A write-only client learns the same facts; availability is not permission.
    let other = grant(&mut s, 1, 0, rustic_abi::files::WRITE_RIGHT, 0);
    let restricted = run(&mut s, &mut d, 1, ask(other), 1);
    assert_eq!(restricted.status, 0);
    assert_eq!(restricted.context, other);
    assert_eq!(
        Capabilities::decode(&restricted).unwrap(),
        Capabilities::decode(&reply).unwrap()
    );
    // Revoked and expired clients are refused like any other request.
    s.revoke(0).unwrap();
    assert_eq!(
        run(&mut s, &mut d, 0, ask(context), 1).status,
        Error::Revoked as u8
    );
    assert!(s.volume.stat(file).is_ok());
}

#[test]
fn malformed_discovery_requests_and_replies_are_rejected() {
    let (mut s, mut d, _, _) = setup();
    let context = grant(&mut s, 0, 0, READ_RIGHT, 0);
    for mutate in [
        |p: &mut Packet| p.id = 1,
        |p: &mut Packet| p.arg = 1,
        |p: &mut Packet| p.version = 1,
        |p: &mut Packet| p.count = 1,
    ] {
        let mut request = ask(context);
        mutate(&mut request);
        assert_eq!(
            run(&mut s, &mut d, 0, request, 1).status,
            Error::Protocol as u8
        );
    }
    let good = run(&mut s, &mut d, 0, ask(context), 1);
    assert!(Capabilities::decode(&good).is_ok());
    // The wire boundary must carry this opcode, not just the service.
    let wire = Packet::decode(&ask(context).encode()).unwrap();
    assert_eq!(wire.op, CAPABILITIES);
    let reply = Packet::decode(&good.encode()).unwrap();
    assert_eq!(Capabilities::decode(&reply), Capabilities::decode(&good));
    for mutate in [
        |p: &mut Packet| p.data[0] = 4,
        |p: &mut Packet| p.data[0] = 0,
        |p: &mut Packet| p.count = METHODS as u8 + 1,
        |p: &mut Packet| p.version = 2,
        |p: &mut Packet| p.data[METHODS] = 1,
        |p: &mut Packet| p.arg = 0,
        |p: &mut Packet| p.arg |= 0xffff,
    ] {
        let mut reply = good;
        mutate(&mut reply);
        assert_eq!(Capabilities::decode(&reply), Err(Error::Protocol));
    }
}
