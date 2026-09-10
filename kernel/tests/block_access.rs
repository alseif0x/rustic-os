// SPDX-License-Identifier: Apache-2.0
use rustic_abi::block::{ALL, Completion, Effect, Error, Operation, READ, SECTOR, Status};
use rustic_kernel::block::{
    Geometry,
    access::{Broker, Grant},
};
const DISK: Geometry = Geometry {
    sectors: 100,
    read_only: false,
};
fn grant(broker: &mut Broker, owner: u64) -> u64 {
    broker
        .grant(
            owner,
            Grant {
                first: 0,
                sectors: 100,
                rights: ALL,
            },
            100,
        )
        .unwrap()
}
fn submit(
    broker: &mut Broker,
    owner: u64,
    handle: u64,
    operation: Operation,
) -> Result<u64, Error> {
    broker.admit(owner, handle, operation, 0, [0x35; SECTOR], DISK)
}
#[test]
fn authority_is_owned_scoped_and_separate_from_ipc() {
    let mut broker = Broker::new();
    let handle = broker
        .grant(
            1,
            Grant {
                first: 8,
                sectors: 2,
                rights: READ,
            },
            100,
        )
        .unwrap();
    assert_eq!(broker.check(1, handle, Operation::Read, 1, DISK), Ok(9));
    assert_eq!(
        broker.check(1, handle, Operation::Read, 2, DISK),
        Err(Error::Range)
    );
    assert_eq!(
        broker.check(1, handle, Operation::Read, u64::MAX, DISK),
        Err(Error::Range)
    );
    assert_eq!(
        broker.check(1, handle, Operation::Write, 0, DISK),
        Err(Error::Denied)
    );
    assert_eq!(
        broker.check(1, handle, Operation::Flush, 0, DISK),
        Err(Error::Denied)
    );
    assert_eq!(broker.geometry(2, handle, DISK), Err(Error::Handle));
    assert_eq!(
        broker.geometry(1, handle & ((1 << 56) - 1), DISK),
        Err(Error::Handle)
    );
    broker.close(1, handle).unwrap();
    let new = grant(&mut broker, 1);
    assert_ne!(handle, new);
    assert_eq!(broker.geometry(1, handle, DISK), Err(Error::Handle));
    assert_eq!(
        broker.check(
            1,
            new,
            Operation::Write,
            0,
            Geometry {
                read_only: true,
                ..DISK
            }
        ),
        Err(Error::ReadOnly)
    );
    assert_eq!(
        broker.check(
            1,
            new,
            Operation::Flush,
            0,
            Geometry {
                read_only: true,
                ..DISK
            }
        ),
        Err(Error::ReadOnly)
    );
    for invalid in [
        Grant {
            first: 1,
            sectors: 100,
            rights: READ,
        },
        Grant {
            first: u64::MAX,
            sectors: 2,
            rights: READ,
        },
        Grant {
            first: 0,
            sectors: 0,
            rights: READ,
        },
    ] {
        assert_eq!(broker.grant(2, invalid, 100), Err(Error::Range));
    }
    assert_eq!(
        broker.grant(
            2,
            Grant {
                first: 8,
                sectors: 2,
                rights: ALL
            },
            100
        ),
        Err(Error::Denied)
    );
    assert_eq!(
        broker.grant(
            2,
            Grant {
                first: 0,
                sectors: 100,
                rights: 8
            },
            100
        ),
        Err(Error::Denied)
    );
}
#[test]
fn quotas_fifo_and_result_retention_bound_memory() {
    let mut broker = Broker::new();
    let a = grant(&mut broker, 1);
    let b = grant(&mut broker, 2);
    let c = grant(&mut broker, 3);
    let _d = grant(&mut broker, 4);
    assert_eq!(
        broker.grant(
            5,
            Grant {
                first: 0,
                sectors: 100,
                rights: READ
            },
            100
        ),
        Err(Error::Quota)
    );
    assert_eq!(
        broker.grant(
            1,
            Grant {
                first: 0,
                sectors: 100,
                rights: READ
            },
            100
        ),
        Err(Error::Quota)
    );
    let first = submit(&mut broker, 1, a, Operation::Read).unwrap();
    assert_eq!(submit(&mut broker, 1, a, Operation::Read), Err(Error::Busy));
    let second = submit(&mut broker, 2, b, Operation::Write).unwrap();
    assert_eq!(submit(&mut broker, 3, c, Operation::Read), Err(Error::Busy));
    assert_eq!(broker.wait(1, a, first), Err(Error::WouldBlock));
    assert_eq!(broker.wait(1, a, second), Err(Error::NoRequest));
    assert_eq!(broker.start().unwrap().id, first);
    assert!(broker.start().is_none());
    broker.finish(first, Status::Success, [0x71; SECTOR]);
    let result = broker.peek(1, a).unwrap();
    assert_eq!(result.data, [0x71; SECTOR]);
    assert_eq!(broker.peek(1, a), Ok(result));
    assert_eq!(submit(&mut broker, 3, c, Operation::Read), Err(Error::Busy));
    broker.consume(1, a).unwrap();
    assert_eq!(broker.peek(1, a), Err(Error::NoRequest));
    let third = submit(&mut broker, 3, c, Operation::Read).unwrap();
    let active = broker.start().unwrap();
    assert_eq!((active.id, active.data), (second, [0x35; SECTOR]));
    broker.finish(second, Status::Io, [0xff; SECTOR]);
    let result = broker.peek(2, b).unwrap();
    assert_eq!(result.effect, Effect::Unknown);
    assert_eq!(result.data, [0; SECTOR]);
    assert!(Completion::decode(&result.encode()).is_ok());
    assert_eq!(broker.start().unwrap().id, third);
}
#[test]
fn queued_cancellation_prevents_submission_and_active_cancellation_does_not_lie() {
    let mut broker = Broker::new();
    let a = grant(&mut broker, 1);
    let id = submit(&mut broker, 1, a, Operation::Write).unwrap();
    assert_eq!(broker.cancel(1, a, id + 1), Err(Error::NoRequest));
    assert_eq!(broker.cancel(1, a, id), Ok(0));
    assert!(broker.start().is_none());
    let result = broker.peek(1, a).unwrap();
    assert_eq!(
        (result.status, result.effect),
        (Status::Cancelled, Effect::None)
    );
    assert_eq!(result.data, [0; SECTOR]);
    broker.consume(1, a).unwrap();
    let id = submit(&mut broker, 1, a, Operation::Write).unwrap();
    broker.start().unwrap();
    assert_eq!(broker.cancel(1, a, id), Ok(1));
    broker.finish(id, Status::Timeout, [0xff; SECTOR]);
    assert_eq!(broker.peek(1, a).unwrap().effect, Effect::Unknown);
    assert_eq!(broker.cancel(1, a, id), Ok(1));
}
#[test]
fn dead_owner_cannot_release_active_storage_or_recover_authority() {
    let mut broker = Broker::new();
    let a = grant(&mut broker, 1);
    let b = grant(&mut broker, 2);
    let first = submit(&mut broker, 1, a, Operation::Read).unwrap();
    broker.start().unwrap();
    submit(&mut broker, 2, b, Operation::Write).unwrap();
    broker.close_owner(1);
    broker.close_owner(2);
    assert_eq!(broker.counts(), (0, 1));
    assert_eq!(broker.active(), Some(first));
    assert!(broker.start().is_none());
    let new = grant(&mut broker, 3);
    assert_eq!(broker.peek(3, a), Err(Error::Handle));
    let second = submit(&mut broker, 3, new, Operation::Read).unwrap();
    broker.finish(first, Status::Timeout, [0xff; SECTOR]);
    assert_eq!(broker.counts(), (1, 1));
    assert_eq!(broker.start().unwrap().id, second);
    broker.finish(second, Status::Unavailable, [0xff; SECTOR]);
    let result = broker.peek(3, new).unwrap();
    assert_eq!(
        (result.status, result.effect, result.data),
        (Status::Unavailable, Effect::None, [0; SECTOR])
    );
    broker.close_owner(3);
    assert_eq!(broker.counts(), (0, 0));
}
