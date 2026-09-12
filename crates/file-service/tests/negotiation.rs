// SPDX-License-Identifier: Apache-2.0
mod support;
use rustic_abi::{
    files::{
        CANCEL_RIGHT, Error, Packet, READ_RIGHT,
        negotiation::{self as n, Descriptor},
    },
    services::{Availability, Method},
};
use rustic_fs::Disk;
use support::{grant, run, setup};

struct NoIo;
impl Disk for NoIo {
    fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), rustic_fs::Error> {
        panic!("discovery read disk")
    }
    fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), rustic_fs::Error> {
        panic!("discovery wrote disk")
    }
    fn flush(&mut self) -> Result<(), rustic_fs::Error> {
        panic!("discovery flushed disk")
    }
}
fn ask(s: &mut rustic_file_service::Server, context: u32, method: Method) -> Packet {
    s.handle(&mut NoIo, 0, 10, n::request(method, context).unwrap(), 1)
}

#[test]
fn support_tracks_v3_v4_v5_without_io_or_allocating_an_origin() {
    let (mut s, mut d, _, _) = setup();
    let context = grant(&mut s, 0, 0, READ_RIGHT, 0);
    for phase in 0..4 {
        match phase {
            1 => {
                s.volume.enable_recovery(&mut d, [7; 16]).unwrap();
                s.volume.enable_operations(&mut d).unwrap();
            }
            2 => s.volume.enable_admissions(&mut d).unwrap(),
            3 => s.volume.enable_prevention_reasons(&mut d).unwrap(),
            _ => (),
        }
        let sequence = s.volume.sequence();
        for method in [Method::OperationsGet, Method::OperationsCancel] {
            let reply = ask(&mut s, context, method);
            assert_eq!(reply.status, 0);
            let descriptor = Descriptor::decode(&reply, method).unwrap();
            assert_eq!(
                descriptor.availability,
                if phase < 2 {
                    Availability::Unavailable
                } else {
                    Availability::Available
                }
            );
            assert_eq!(
                (
                    descriptor.limits.retained_operations,
                    descriptor.limits.execution_tickets,
                    descriptor.limits.active_publications
                ),
                (2, 2, 1)
            );
        }
        assert_eq!(s.volume.sequence(), sequence);
        // The old v1 slot is still v1, even when v2 cancellation is available.
        assert_eq!(
            s.capabilities().of(Method::OperationsCancel),
            Availability::Unavailable
        );
    }
}

#[test]
fn discovery_rechecks_peer_context_revocation_expiry_and_does_not_grant_inspection() {
    let (mut s, mut d, file, _) = setup();
    let context = s
        .grant(
            0,
            rustic_file_service::Grant {
                peer: 10,
                endpoint: 1,
                scope: file,
                rights: CANCEL_RIGHT,
                generation: 0,
                expires: 10,
                subject: 9,
            },
        )
        .unwrap();
    assert_eq!(ask(&mut s, context, Method::OperationsGet).status, 0);
    let p = n::request(Method::OperationsGet, context).unwrap();
    assert_eq!(s.handle(&mut NoIo, 0, 11, p, 1).status, Error::Denied as u8);
    assert_eq!(
        ask(&mut s, context + 1, Method::OperationsGet).status,
        Error::Revoked as u8
    );
    assert_eq!(
        s.handle(&mut NoIo, 0, 10, p, 10).status,
        Error::Expired as u8
    );
    s.revoke(0).unwrap();
    assert_eq!(
        ask(&mut s, context, Method::OperationsGet).status,
        Error::Revoked as u8
    );
    let mut p = p;
    p.version = 1;
    assert_eq!(
        run(&mut s, &mut d, 0, p, 1).status,
        Error::UnsupportedVersion as u8
    );
}

#[test]
fn a_poisoned_volume_cannot_advertise_healthy_support() {
    struct Fail;
    impl Disk for Fail {
        fn read(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), rustic_fs::Error> {
            Err(rustic_fs::Error::Io)
        }
        fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), rustic_fs::Error> {
            Err(rustic_fs::Error::Io)
        }
        fn flush(&mut self) -> Result<(), rustic_fs::Error> {
            Err(rustic_fs::Error::Io)
        }
    }
    let (mut s, mut d, _, _) = setup();
    s.volume.enable_recovery(&mut d, [7; 16]).unwrap();
    s.volume.enable_operations(&mut d).unwrap();
    s.volume.enable_admissions(&mut d).unwrap();
    assert!(
        s.volume
            .create(&mut Fail, 4, b"failed", rustic_fs::Kind::File)
            .is_err()
    );
    let context = grant(&mut s, 0, 0, READ_RIGHT, 0);
    let reply = ask(&mut s, context, Method::OperationsGet);
    assert_eq!(reply.status, Error::Uncertain as u8);
    assert_eq!(reply.data, [0; 40]);
}
