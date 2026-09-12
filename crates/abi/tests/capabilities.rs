// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::capabilities::{Bounds, Capabilities};
use rustic_abi::files::{CAPABILITIES, Error, MAX_INLINE, Packet};
use rustic_abi::services::{Availability, METHODS, Method};

fn report() -> Capabilities {
    let mut availability = [Availability::Unavailable; METHODS];
    availability[Method::CapabilitiesList as usize - 1] = Availability::Degraded;
    availability[Method::FilesRead as usize - 1] = Availability::Available;
    Capabilities {
        availability,
        bounds: Bounds {
            max_inline_bytes: MAX_INLINE as u16,
            max_page_items: METHODS as u8,
            receipt_capacity: 2,
        },
    }
}

#[test]
fn discovery_survives_the_wire_and_keeps_the_catalog_identity() {
    let packet = report().packet(9).unwrap();
    // The shared decoder must accept this opcode, or the request never arrives.
    let wire = Packet::decode(&packet.encode()).unwrap();
    assert_eq!(wire.op, CAPABILITIES);
    let decoded = Capabilities::decode(&wire).unwrap();
    assert_eq!(decoded, report());
    assert_eq!(decoded.of(Method::FilesRead), Availability::Available);
    assert_eq!(decoded.of(Method::SystemStatus), Availability::Unavailable);
    assert_eq!(
        Method::decode(Method::OperationsGet as u8),
        Some(Method::OperationsGet)
    );
    assert_eq!(Method::decode(0), None);
    assert_eq!(Method::decode(METHODS as u8 + 1), None);
    assert_eq!(Method::FilesReplace.name(), "files.replace");
    assert_eq!(Availability::Degraded.name(), "degraded");
}

#[test]
fn malformed_reports_and_impossible_bounds_are_rejected() {
    let good = report().packet(9).unwrap();
    for mutate in [
        |p: &mut Packet| p.op = rustic_abi::files::STAT,
        |p: &mut Packet| p.id = 1,
        |p: &mut Packet| p.status = 1,
        |p: &mut Packet| p.count = METHODS as u8 - 1,
        |p: &mut Packet| p.version = 2,
        |p: &mut Packet| p.data[0] = 9,
        |p: &mut Packet| p.data[METHODS] = 1,
        |p: &mut Packet| p.arg = 0,
    ] {
        let mut packet = good;
        mutate(&mut packet);
        assert_eq!(Capabilities::decode(&packet), Err(Error::Protocol));
    }
    for bounds in [
        Bounds {
            max_inline_bytes: MAX_INLINE as u16 + 1,
            max_page_items: METHODS as u8,
            receipt_capacity: 2,
        },
        Bounds {
            max_inline_bytes: MAX_INLINE as u16,
            max_page_items: 0,
            receipt_capacity: 2,
        },
        Bounds {
            max_inline_bytes: MAX_INLINE as u16,
            max_page_items: METHODS as u8,
            receipt_capacity: 0,
        },
    ] {
        let invalid = Capabilities { bounds, ..report() };
        assert_eq!(invalid.packet(9), Err(Error::Protocol));
    }
}
