// SPDX-License-Identifier: Apache-2.0
//! Empty content, receipt retention and service restart share one recovery contract.
use super::{authorize, lookup, support::*};
use rustic_abi::files::{recovery::Receipt, recovery::Retry, *};
use rustic_file_service::Server;
use rustic_fs::{Disk, Node, Volume};

fn replace(
    server: &mut Server,
    disk: &mut impl Disk,
    context: u32,
    id: u32,
    version: u64,
    retry: Retry,
    bytes: &[u8],
) -> Packet {
    let mut begin = request(TRACK_BEGIN, id, context);
    begin.version = version;
    begin.arg = bytes.len() as u32;
    begin.count = 32;
    begin.data[..32].copy_from_slice(&retry.encode());
    let reply = server.handle(disk, 0, 10, begin, 1);
    if reply.status != 0 {
        return reply;
    }
    for (index, bytes) in bytes.chunks(DATA).enumerate() {
        let mut chunk = request(CHUNK, id, context);
        chunk.arg = (index * DATA) as u32;
        chunk.count = bytes.len() as u8;
        chunk.data[..bytes.len()].copy_from_slice(bytes);
        assert_eq!(server.handle(disk, 0, 10, chunk, 1).status, 0);
    }
    let reply = server.handle(disk, 0, 10, request(COMMIT, id, context), 1);
    if reply.status != 0 && reply.status != Error::Uncertain as u8 {
        // Match the SDK's cleanup after a definite rejection.
        assert_eq!(
            server
                .handle(disk, 0, 10, request(ABORT, id, context), 1)
                .status,
            0
        );
    }
    reply
}

fn committed(packet: Packet, id: u32, previous: u64, retry: Retry, length: u16) -> Receipt {
    assert_eq!(packet.status, 0);
    let receipt = Receipt::decode(packet).unwrap();
    assert_eq!(receipt.id, id);
    assert_eq!(receipt.previous, previous);
    assert!(receipt.committed > previous);
    assert_eq!(receipt.retry, retry);
    assert_eq!(receipt.length, length);
    receipt
}

fn unchanged_file(actual: Node, expected: Node) {
    assert_eq!(
        (actual.id, actual.version, actual.length),
        (expected.id, expected.version, expected.length)
    );
    assert_eq!(
        (actual.parent, actual.kind, actual.space, actual.name()),
        (
            expected.parent,
            expected.kind,
            expected.space,
            expected.name()
        )
    );
}

/// Reach the exact retention boundary before the CI mission's final replacement.
fn rotated_empty() -> (Server, Memory, Node, Retry, Retry, u32) {
    let (mut server, mut disk, id, other) = setup();
    server.volume.enable_recovery(&mut disk, [7; 16]).unwrap();
    let version = server.volume.stat(id).unwrap().version;
    let before = server
        .volume
        .replace(&mut disk, id, version, b"before")
        .unwrap();
    let version = server.volume.stat(other).unwrap().version;
    server
        .volume
        .replace(&mut disk, other, version, b"untouched")
        .unwrap();
    let context = authorize(&mut server, 0, id, 7, 9, 0);
    let old = Retry {
        lineage: [7; 16],
        epoch: 1,
        key: 77,
    };
    let first = committed(
        replace(
            &mut server,
            &mut disk,
            context,
            id,
            before.version,
            old,
            b"reply deliberately unobserved",
        ),
        id,
        before.version,
        old,
        29,
    );
    let human = server
        .volume
        .replace(&mut disk, id, first.committed, b"human-edit")
        .unwrap();
    let second = Retry { key: 78, ..old };
    assert_eq!(
        replace(
            &mut server,
            &mut disk,
            context,
            id,
            before.version,
            second,
            b"stale",
        )
        .status,
        Error::Version as u8
    );
    let empty_receipt = committed(
        replace(
            &mut server,
            &mut disk,
            context,
            id,
            human.version,
            second,
            b"",
        ),
        id,
        human.version,
        second,
        0,
    );
    let empty = server.volume.stat(id).unwrap();
    assert_eq!((empty.length, empty.version), (0, empty_receipt.committed));
    assert_eq!(
        replace(
            &mut server,
            &mut disk,
            context,
            id,
            empty.version,
            Retry { key: 88, ..old },
            b"full",
        )
        .status,
        Error::Full as u8
    );
    unchanged_file(server.volume.stat(id).unwrap(), empty);
    assert_eq!(server.volume.advance_epoch(&mut disk), Ok(2));
    assert_eq!(
        run(&mut server, &mut disk, 0, lookup(id, context, old), 1).status,
        Error::ExpiredEpoch as u8
    );
    assert_eq!(
        replace(
            &mut server,
            &mut disk,
            context,
            id,
            empty.version,
            old,
            b"stale-key",
        )
        .status,
        Error::ExpiredEpoch as u8
    );
    unchanged_file(server.volume.stat(id).unwrap(), empty);
    let next = Retry {
        epoch: 2,
        key: 42,
        ..old
    };
    (server, disk, empty, old, next, context)
}

fn remount(disk: &mut Memory, empty: Node, old: Retry, next: Retry) -> Option<Receipt> {
    let mut server = Server::new(Volume::mount(disk).unwrap());
    let context = authorize(&mut server, 0, empty.id, INSPECT_RIGHT, 9, 0);
    assert_eq!(server.volume.recovery_info(), Ok((next.lineage, 2)));
    assert_eq!(
        run(&mut server, disk, 0, lookup(empty.id, context, old), 1).status,
        Error::ExpiredEpoch as u8
    );
    let packet = run(&mut server, disk, 0, lookup(empty.id, context, next), 1);
    let mut bytes = [0; 32];
    let length = server.volume.read(disk, empty.id, 0, &mut bytes).unwrap();
    if packet.status == Error::OutcomeUnknown as u8 {
        assert_eq!(length, 0);
        unchanged_file(server.volume.stat(empty.id).unwrap(), empty);
        None
    } else {
        let receipt = committed(packet, empty.id, empty.version, next, 5);
        assert_eq!(&bytes[..length], b"after");
        assert_eq!(
            server.volume.stat(empty.id).unwrap().version,
            receipt.committed
        );
        // Inspection-only authority may replay the durable receipt after restart.
        assert_eq!(
            Receipt::decode(replace(
                &mut server,
                disk,
                context,
                empty.id,
                empty.version,
                next,
                b"after",
            )),
            Ok(receipt)
        );
        Some(receipt)
    }
}

#[test]
fn empty_second_receipt_full_rotation_then_new_epoch_commit_survives_restart() {
    let (mut server, mut disk, empty, old, next, context) = rotated_empty();
    let receipt = committed(
        replace(
            &mut server,
            &mut disk,
            context,
            empty.id,
            empty.version,
            next,
            b"after",
        ),
        empty.id,
        empty.version,
        next,
        5,
    );
    assert_eq!(remount(&mut disk.clone(), empty, old, next), Some(receipt));
}

/// A failed write/flush may have taken effect; only a successful flush promises durability.
struct CutDisk {
    live: Memory,
    durable: Memory,
    operations: usize,
    fail: Option<usize>,
    apply_failed: bool,
}
impl CutDisk {
    fn new(disk: Memory, fail: Option<usize>, apply_failed: bool) -> Self {
        Self {
            live: disk.clone(),
            durable: disk,
            operations: 0,
            fail,
            apply_failed,
        }
    }
    fn step(&mut self) -> bool {
        let failed = self.fail == Some(self.operations);
        self.operations += 1;
        failed
    }
}
impl Disk for CutDisk {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), rustic_fs::Error> {
        self.live.read(sector, bytes)
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), rustic_fs::Error> {
        let failed = self.step();
        if !failed || self.apply_failed {
            self.live.write(sector, bytes)?;
        }
        if failed {
            Err(rustic_fs::Error::Io)
        } else {
            Ok(())
        }
    }
    fn flush(&mut self) -> Result<(), rustic_fs::Error> {
        let failed = self.step();
        if !failed || self.apply_failed {
            self.durable = self.live.clone();
        }
        if failed {
            Err(rustic_fs::Error::Io)
        } else {
            Ok(())
        }
    }
}

#[test]
fn post_rotation_write_failures_recover_empty_or_content_and_receipt_together() {
    let (mut server, disk, empty, old, next, context) = rotated_empty();
    let mut probe = CutDisk::new(disk.clone(), None, false);
    let receipt = committed(
        replace(
            &mut server,
            &mut probe,
            context,
            empty.id,
            empty.version,
            next,
            b"after",
        ),
        empty.id,
        empty.version,
        next,
        5,
    );
    assert!(probe.operations > 0);
    assert_eq!(
        remount(&mut probe.durable.clone(), empty, old, next),
        Some(receipt)
    );
    for cut in 0..probe.operations {
        for apply_failed in [false, true] {
            let mut disk = CutDisk::new(disk.clone(), Some(cut), apply_failed);
            let mut server = Server::new(Volume::mount(&mut disk).unwrap());
            let context = authorize(&mut server, 0, empty.id, 7, 9, 0);
            assert_eq!(
                replace(
                    &mut server,
                    &mut disk,
                    context,
                    empty.id,
                    empty.version,
                    next,
                    b"after",
                )
                .status,
                Error::Uncertain as u8,
                "cut={cut} apply_failed={apply_failed}"
            );
            assert!(matches!(
                server.volume.stat(empty.id),
                Err(rustic_fs::Error::Uncertain)
            ));
            // Cover loss of unflushed writes and retention of writes already sent.
            for mut recovered in [disk.durable.clone(), disk.live.clone()] {
                let _ = remount(&mut recovered, empty, old, next);
            }
        }
    }
}
