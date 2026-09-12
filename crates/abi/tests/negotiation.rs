// SPDX-License-Identifier: Apache-2.0
use rustic_abi::{
    files::{
        Error, Packet,
        negotiation::{self as n, Descriptor, Limits},
    },
    services::{Availability, Method},
};

fn descriptor(method: Method) -> Packet {
    Descriptor::reviewed(
        method,
        Availability::Available,
        Limits {
            retained_operations: 2,
            execution_tickets: 2,
            active_publications: 1,
        },
    )
    .unwrap()
    .packet(7)
    .unwrap()
}

#[test]
fn every_wire_boundary_keeps_the_selected_version_method_and_profile() {
    for method in [Method::OperationsGet, Method::OperationsCancel] {
        let request = n::request(method, 7).unwrap();
        assert_eq!(
            n::decode_request(&Packet::decode(&request.encode()).unwrap()),
            Ok(method)
        );
        let reply = Packet::decode(&descriptor(method).encode())
            .unwrap()
            .checked_reply(n::DESCRIBE, 7)
            .unwrap();
        assert_eq!(Descriptor::decode(&reply, method).unwrap().method, method);
        assert_eq!(
            Descriptor::decode(
                &reply,
                if method == Method::OperationsGet {
                    Method::OperationsCancel
                } else {
                    Method::OperationsGet
                }
            ),
            Err(Error::Protocol)
        );
    }
}

#[test]
fn unknown_versions_profiles_methods_and_noncanonical_requests_never_downgrade() {
    for (mutate, error) in [
        (|p: &mut Packet| p.version = 1, Error::UnsupportedVersion),
        (|p: &mut Packet| p.version = 3, Error::UnsupportedVersion),
        (|p: &mut Packet| p.arg = 0, Error::UnsupportedVersion),
        (|p: &mut Packet| p.arg = 2, Error::UnsupportedVersion),
        (|p: &mut Packet| p.id = 261, Error::Unsupported),
        (|p: &mut Packet| p.id = 3, Error::Unsupported),
        (|p: &mut Packet| p.count = 1, Error::Protocol),
        (|p: &mut Packet| p.data[39] = 1, Error::Protocol),
        (|p: &mut Packet| p.status = 1, Error::Protocol),
    ] as [(fn(&mut Packet), Error); 9]
    {
        let mut p = n::request(Method::OperationsGet, 7).unwrap();
        mutate(&mut p);
        assert_eq!(n::decode_request(&p), Err(error));
    }
}

#[test]
fn mismatched_digest_impossible_limits_and_ambiguous_reports_fail_closed() {
    for mutate in [
        |p: &mut Packet| p.op = 58,
        |p: &mut Packet| p.version = 1,
        |p: &mut Packet| p.arg = 2,
        |p: &mut Packet| p.count = 35,
        |p: &mut Packet| p.data[0] = 2,
        |p: &mut Packet| p.data[1] = 0,
        |p: &mut Packet| p.data[2] = 3,
        |p: &mut Packet| p.data[3] = 2,
        |p: &mut Packet| p.data[4] ^= 1,
        |p: &mut Packet| p.data[35] ^= 1,
        |p: &mut Packet| p.data[36] = 1,
    ] as [fn(&mut Packet); 11]
    {
        let mut p = descriptor(Method::OperationsGet);
        mutate(&mut p);
        assert_eq!(
            Descriptor::decode(&p, Method::OperationsGet),
            Err(Error::Protocol)
        );
    }
}
