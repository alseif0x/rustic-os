// SPDX-License-Identifier: Apache-2.0
//! Pure client collection tests. Native peer/transport execution is tested in QEMU.
#[path = "../src/files/range/collector.rs"]
mod collector;
#[path = "../src/files/range/operation.rs"]
mod operation;
use collector::{Collector, read_into};
use core::sync::atomic::{AtomicBool, Ordering};
use operation::{
    Operation, RangeProgress, Transport, is_pending_marker, pending_marker, poll_operation,
};
use rustic_abi::files::{
    Error, Packet, READ_CHUNK, READ_OPEN,
    read::{Header, Request},
    reference::{Epoch, References, Version},
};
use sha2::{Digest, Sha256};

fn request(offset: u64, length: u16) -> Request {
    let refs = References::new([1; 16], 4, 6).unwrap();
    Request {
        workspace: refs.workspace,
        resource: refs.resource,
        expected_version: None,
        offset,
        length,
    }
}

fn header(request: Request, file: &[u8]) -> Packet {
    header_version(request, file, 7)
}

fn header_version(request: Request, file: &[u8], version: u64) -> Packet {
    let start = request.offset as usize;
    let end = file.len().min(start + usize::from(request.length));
    Header {
        id: 6,
        size: file.len() as u64,
        version: Version::new(version).unwrap(),
        range_sha256: Sha256::digest(&file[start..end]).into(),
        retry_epoch: Epoch::new(3).unwrap(),
    }
    .packet(11)
    .unwrap()
}

fn chunk(request: Packet, file: &[u8]) -> Packet {
    let decoded = Request::decode(&request).unwrap();
    assert_eq!(request.op, READ_CHUNK);
    assert_eq!(decoded.expected_version, Some(Version::new(7).unwrap()));
    let mut p = Packet::new(READ_CHUNK);
    p.context = 11;
    p.id = 6;
    p.arg = file.len() as u32;
    p.version = 7;
    p.count = decoded.length as u8;
    let start = decoded.offset as usize;
    p.data[..usize::from(decoded.length)]
        .copy_from_slice(&file[start..start + usize::from(decoded.length)]);
    p
}

fn reply_for(request: Packet, file: &[u8], context: u32) -> Packet {
    let decoded = Request::decode(&request).unwrap();
    if request.op == READ_OPEN {
        let start = decoded.offset as usize;
        let end = file.len().min(start + usize::from(decoded.length));
        Header {
            id: decoded.resource.object(),
            size: file.len() as u64,
            version: Version::new(7).unwrap(),
            range_sha256: Sha256::digest(&file[start..end]).into(),
            retry_epoch: Epoch::new(3).unwrap(),
        }
        .packet(context)
        .unwrap()
    } else {
        chunk(request, file)
    }
}

struct FakeTransport {
    token: u64,
    context: u32,
    file: Vec<u8>,
    in_flight: Option<Packet>,
    marker: Option<Packet>,
    sent: Vec<Packet>,
    send_attempts: usize,
    receive_polls: usize,
    block_send_once: bool,
    delay_receive_once: bool,
    corrupt_chunk: bool,
    corrupt_open_hash: bool,
    server_busy_op: Option<u8>,
    range_id: Option<u64>,
}

impl FakeTransport {
    fn new(file: &[u8]) -> Self {
        Self {
            token: 9,
            context: 11,
            file: file.to_vec(),
            in_flight: None,
            marker: None,
            sent: Vec::new(),
            send_attempts: 0,
            receive_polls: 0,
            block_send_once: false,
            delay_receive_once: false,
            corrupt_chunk: false,
            corrupt_open_hash: false,
            server_busy_op: None,
            range_id: Some(1),
        }
    }

    fn drain(&mut self) -> Result<bool, Error> {
        if !self.marker.is_some_and(is_pending_marker) {
            return Err(Error::Unavailable);
        }
        if self.delay_receive_once {
            self.delay_receive_once = false;
            self.receive_polls += 1;
            return Ok(false);
        }
        self.in_flight = None;
        self.marker = None;
        self.range_id = None;
        Ok(true)
    }

    fn rebind(&mut self, token: u64, context: u32) {
        self.token = token;
        self.context = context;
        self.in_flight = None;
        self.marker = None;
        self.range_id = None;
    }

    fn response(&mut self, request: Packet) -> Packet {
        if self.server_busy_op == Some(request.op) {
            self.server_busy_op = None;
            let mut reply = Packet::new(request.op);
            reply.status = Error::Busy as u8;
            reply.context = self.context;
            return reply;
        }
        let mut reply = reply_for(request, &self.file, self.context);
        if request.op == READ_OPEN && self.corrupt_open_hash {
            reply.data[0] ^= 1;
            self.corrupt_open_hash = false;
        }
        if request.op == READ_CHUNK && self.corrupt_chunk {
            reply.data[0] ^= 1;
            self.corrupt_chunk = false;
        }
        reply
    }
}

impl Transport for FakeTransport {
    fn token(&self) -> u64 {
        self.token
    }

    fn context(&self) -> u32 {
        self.context
    }

    fn active_range_id(&self) -> Option<u64> {
        self.range_id
    }

    fn release_range(&mut self, id: u64) {
        if self.range_id == Some(id) {
            self.range_id = None;
        }
    }

    fn send(&mut self, packet: Packet) -> Result<(), Error> {
        self.send_attempts += 1;
        if self.block_send_once {
            self.block_send_once = false;
            return Err(Error::Busy);
        }
        if self.in_flight.is_some() || self.marker.is_some() {
            return Err(Error::Busy);
        }
        assert!(
            self.sent.len() < 8,
            "fake transport is deliberately bounded"
        );
        self.sent.push(packet);
        self.in_flight = Some(packet);
        self.marker = Some(pending_marker(packet.context, self.range_id.unwrap()));
        Ok(())
    }

    fn receive(&mut self, _op: u8, context: u32) -> Result<Option<Packet>, Error> {
        self.receive_polls += 1;
        if self.marker != Some(pending_marker(context, self.range_id.unwrap())) {
            return Err(Error::Protocol);
        }
        if self.delay_receive_once {
            self.delay_receive_once = false;
            return Ok(None);
        }
        let Some(request) = self.in_flight.take() else {
            return Err(Error::Protocol);
        };
        self.marker = None;
        Ok(Some(self.response(request)))
    }
}

#[test]
fn multi_chunk_progress_pins_every_chunk_and_verifies_partial_ranges() {
    let file: Vec<u8> = (0..1024).map(|i| (i * 37) as u8).collect();
    for (offset, length) in [
        (0, 1024),
        (13, 55),
        (39, 56),
        (1, 63),
        (2, 64),
        (3, 65),
        (1000, 40),
    ] {
        let request = request(offset, length);
        let mut collector = Collector::new(request, 11).unwrap();
        collector.open(header(request, &file)).unwrap();
        let mut next_offset = offset;
        while let Some(p) = collector.next().unwrap() {
            let decoded = Request::decode(&p).unwrap();
            assert_eq!(decoded.offset, next_offset);
            assert_eq!(decoded.expected_version, Some(Version::new(7).unwrap()));
            next_offset += u64::from(decoded.length);
            collector.chunk(chunk(p, &file)).unwrap();
        }
        let verified = collector.finish().unwrap();
        let info = verified.info();
        let end = file.len().min(offset as usize + usize::from(length));
        assert_eq!(verified.bytes(), &file[offset as usize..end]);
        assert_eq!(info.eof, end == file.len());
    }
}

#[test]
fn synchronous_read_into_clears_failure_and_undersized_outputs() {
    let request = request(0, 3);
    let file = b"abc";
    let mut output = [0xa5; 8];
    let mut calls = 0;
    let info = read_into(request, 11, &mut output, |packet| {
        calls += 1;
        Ok(reply_for(packet, file, 11))
    })
    .unwrap();
    assert_eq!(info.length, 3);
    assert_eq!(&output[..3], file);
    assert!(output[3..].iter().all(|byte| *byte == 0));
    assert_eq!(calls, 2);

    let mut small = [0xa5; 2];
    let mut calls = 0;
    assert_eq!(
        read_into(request, 11, &mut small, |_| {
            calls += 1;
            Err(Error::Protocol)
        }),
        Err(Error::Size)
    );
    assert_eq!(small, [0; 2]);
    assert_eq!(calls, 0);

    let mut invalid = request;
    invalid.length = 1025;
    let mut output = [0xa5; 2];
    assert_eq!(
        read_into(invalid, 11, &mut output, |_| unreachable!()),
        Err(Error::Invalid)
    );
    assert_eq!(output, [0; 2]);

    let mut failed = [0xa5; 8];
    let result = read_into(request, 11, &mut failed, |packet| {
        let mut reply = reply_for(packet, file, 11);
        if packet.op == READ_CHUNK {
            reply.data[0] ^= 1;
        }
        Ok(reply)
    });
    assert_eq!(result, Err(Error::Protocol));
    assert_eq!(failed, [0; 8]);
}

#[test]
fn pollable_read_uses_one_transport_step_and_pins_each_chunk() {
    let file: Vec<u8> = (0..95).map(|index| (index * 19) as u8).collect();
    let request = request(0, 95);
    let mut operation = Operation::new(request, 9, 11, 1).unwrap();
    let mut transport = FakeTransport::new(&file);
    transport.delay_receive_once = true;

    let mut done = false;
    for _ in 0..16 {
        let before = transport.send_attempts + transport.receive_polls;
        let progress = poll_operation(&mut operation, &mut transport);
        let steps = transport.send_attempts + transport.receive_polls - before;
        assert_eq!(steps, 1, "each call attempts one send or receive poll");
        match progress {
            Ok(RangeProgress::Pending) => {}
            Ok(RangeProgress::Complete) => {
                done = true;
                break;
            }
            Err(error) => panic!("unexpected range failure: {error:?}"),
        }
    }
    assert!(done);
    assert_eq!(transport.active_range_id(), None);
    assert_eq!(transport.sent.len(), 4);
    for packet in &transport.sent[1..] {
        let decoded = Request::decode(packet).unwrap();
        assert_eq!(decoded.expected_version, Some(Version::new(7).unwrap()));
    }
    let verified = operation.finish().unwrap();
    assert_eq!(verified.bytes(), &file);
}

#[test]
fn pollable_send_backpressure_never_admits_or_retries_implicitly() {
    static DROP_CLEARED: AtomicBool = AtomicBool::new(false);
    let mut operation = Operation::new(request(0, 80), 9, 11, 1).unwrap();
    let mut transport = FakeTransport::new(&[0x4a; 80]);
    transport.block_send_once = true;

    assert_eq!(
        poll_operation(&mut operation, &mut transport),
        Err(Error::Busy)
    );
    assert_eq!(transport.send_attempts, 1);
    assert!(transport.sent.is_empty());
    assert_eq!(transport.active_range_id(), Some(1));
    assert_eq!(
        poll_operation(&mut operation, &mut transport),
        Ok(RangeProgress::Pending)
    );
    assert_eq!(transport.send_attempts, 2);
    assert_eq!(transport.sent.len(), 1);

    DROP_CLEARED.store(false, Ordering::Relaxed);
    operation.observe_drop(&DROP_CLEARED);
    drop(operation);
    assert!(transport.marker.is_some_and(is_pending_marker));
    transport.delay_receive_once = true;
    assert_eq!(transport.drain(), Ok(false));
    assert!(transport.marker.is_some_and(is_pending_marker));
    assert_eq!(transport.drain(), Ok(true));
    assert!(transport.marker.is_none());
    assert!(DROP_CLEARED.load(Ordering::Relaxed));
}

#[test]
fn canonical_server_busy_errors_release_terminal_open_and_chunk_leases() {
    for busy_op in [READ_OPEN, READ_CHUNK] {
        let mut transport = FakeTransport::new(&[0x51; 80]);
        transport.server_busy_op = Some(busy_op);
        let mut operation = Operation::new(request(0, 80), 9, 11, 1).unwrap();
        let error = loop {
            match poll_operation(&mut operation, &mut transport) {
                Ok(RangeProgress::Pending) => {}
                Ok(RangeProgress::Complete) => panic!("server Busy response completed a read"),
                Err(error) => break error,
            }
        };
        assert_eq!(error, Error::Busy);
        assert_eq!(transport.active_range_id(), None);
        assert_eq!(
            poll_operation(&mut operation, &mut transport),
            Err(Error::Busy)
        );
        assert_eq!(transport.active_range_id(), None);

        transport.range_id = Some(2);
        let mut next = Operation::new(request(0, 80), 9, 11, 2).unwrap();
        let completed = loop {
            match poll_operation(&mut next, &mut transport) {
                Ok(RangeProgress::Pending) => {}
                Ok(RangeProgress::Complete) => break true,
                Err(_) => break false,
            }
        };
        assert!(completed);
    }
}

#[test]
fn drained_stale_range_cannot_consume_the_next_ranges_reply() {
    let file = [0x37; 80];
    let mut transport = FakeTransport::new(&file);
    let mut old = Operation::new(request(0, 80), 9, 11, 1).unwrap();
    assert_eq!(
        poll_operation(&mut old, &mut transport),
        Ok(RangeProgress::Pending)
    );
    assert_eq!(transport.drain(), Ok(true));

    transport.range_id = Some(2);
    let mut current = Operation::new(request(0, 80), 9, 11, 2).unwrap();
    assert_eq!(
        poll_operation(&mut current, &mut transport),
        Ok(RangeProgress::Pending)
    );
    let polls = transport.receive_polls;

    assert_eq!(
        poll_operation(&mut old, &mut transport),
        Err(Error::Unavailable)
    );
    assert_eq!(transport.receive_polls, polls);
    assert_eq!(transport.active_range_id(), Some(2));
    assert_eq!(transport.marker.unwrap().version, 2);
    assert_eq!(
        poll_operation(&mut current, &mut transport),
        Ok(RangeProgress::Pending)
    );
}

#[test]
fn foreign_binding_mismatch_does_not_release_another_clients_range() {
    let mut operation = Operation::new(request(0, 80), 9, 11, 1).unwrap();
    let mut transport = FakeTransport::new(&[0x12; 80]);
    assert_eq!(
        poll_operation(&mut operation, &mut transport),
        Ok(RangeProgress::Pending)
    );

    transport.rebind(10, 12);
    transport.range_id = Some(1);
    assert_eq!(
        poll_operation(&mut operation, &mut transport),
        Err(Error::Unavailable)
    );
    assert_eq!(transport.active_range_id(), Some(1));
}

#[test]
fn pollable_read_fails_and_clears_when_binding_changes_or_range_hash_fails() {
    static DROP_CLEARED: AtomicBool = AtomicBool::new(false);
    DROP_CLEARED.store(false, Ordering::Relaxed);
    let mut operation = Operation::new(request(0, 80), 9, 11, 1).unwrap();
    let mut transport = FakeTransport::new(&[0x72; 80]);
    operation.observe_drop(&DROP_CLEARED);
    assert_eq!(
        poll_operation(&mut operation, &mut transport),
        Ok(RangeProgress::Pending)
    );
    transport.rebind(10, 12);
    assert_eq!(
        poll_operation(&mut operation, &mut transport),
        Err(Error::Unavailable)
    );
    assert!(DROP_CLEARED.load(Ordering::Relaxed));

    DROP_CLEARED.store(false, Ordering::Relaxed);
    let mut operation = Operation::new(request(0, 80), 9, 11, 1).unwrap();
    let mut transport = FakeTransport::new(&[0x72; 80]);
    transport.corrupt_open_hash = true;
    operation.observe_drop(&DROP_CLEARED);
    let error = loop {
        match poll_operation(&mut operation, &mut transport) {
            Ok(RangeProgress::Pending) => {}
            Ok(RangeProgress::Complete) => panic!("corrupt range hash was accepted"),
            Err(error) => break error,
        }
    };
    assert_eq!(error, Error::Protocol);
    assert!(transport.marker.is_none());
    assert!(DROP_CLEARED.load(Ordering::Relaxed));
}

#[test]
fn pinned_open_rejects_a_different_version_before_returning_bytes() {
    let file = b"versioned";
    let mut request = request(0, 9);
    request.expected_version = Some(Version::new(7).unwrap());
    let mut collector = Collector::new(request, 11).unwrap();
    assert_eq!(
        collector.open(header_version(request, file, 8)),
        Err(Error::Protocol)
    );
    assert!(collector.buffered().iter().all(|byte| *byte == 0));
}

#[test]
fn empty_and_eof_reads_validate_the_empty_hash_without_a_data_packet() {
    for file in [b"".as_slice(), b"abc".as_slice()] {
        let request = request(file.len() as u64, 1);
        let mut collector = Collector::new(request, 11).unwrap();
        collector.open(header(request, file)).unwrap();
        assert_eq!(collector.next().unwrap(), None);
        let verified = collector.finish().unwrap();
        assert_eq!((verified.info().length, verified.info().eof), (0, true));
        assert!(verified.bytes().is_empty());
    }
}

#[test]
fn incomplete_reads_clear_collected_data_on_failure_and_drop() {
    static DROP_CLEARED: AtomicBool = AtomicBool::new(false);
    for abandon in [false, true] {
        let file = [0x5a; 80];
        let request = request(0, 80);
        let mut collector = Collector::new(request, 11).unwrap();
        collector.open(header(request, &file)).unwrap();
        collector
            .chunk(chunk(collector.next().unwrap().unwrap(), &file))
            .unwrap();
        assert!(collector.buffered()[..40].iter().any(|byte| *byte != 0));
        DROP_CLEARED.store(false, Ordering::Relaxed);
        collector.observe_drop(&DROP_CLEARED);
        if abandon {
            drop(collector);
        } else {
            assert_eq!(collector.finish(), Err(Error::Protocol));
        }
        assert!(DROP_CLEARED.load(Ordering::Relaxed));
    }
}

#[test]
fn changed_version_or_malformed_chunk_fails_and_clears_the_whole_buffer() {
    let file: Vec<u8> = (0..80).collect();
    for mutation in 0..8 {
        let request = request(0, 79);
        let mut collector = Collector::new(request, 11).unwrap();
        collector.open(header(request, &file)).unwrap();
        collector
            .chunk(chunk(collector.next().unwrap().unwrap(), &file))
            .unwrap();
        let mut bad = chunk(collector.next().unwrap().unwrap(), &file);
        match mutation {
            0 => bad.version += 1,
            1 => bad.arg += 1,
            2 => bad.id += 1,
            3 => bad.context += 1,
            4 => bad.count = 0,
            5 => bad.count = 41,
            6 => bad.op = 6,
            _ => bad.data[39] = 1,
        }
        assert_eq!(collector.chunk(bad), Err(Error::Protocol));
        assert!(collector.buffered().iter().all(|byte| *byte == 0));
        assert_eq!(collector.finish(), Err(Error::Protocol));
    }
}

#[test]
fn corrupt_hash_repeated_data_and_malformed_headers_never_verify() {
    let file: Vec<u8> = (0..80).collect();
    for corrupt_hash in [false, true] {
        let request = request(0, 80);
        let mut collector = Collector::new(request, 11).unwrap();
        let mut open = header(request, &file);
        if corrupt_hash {
            open.data[0] ^= 1;
        }
        collector.open(open).unwrap();
        let first = chunk(collector.next().unwrap().unwrap(), &file);
        collector.chunk(first).unwrap();
        let second = if corrupt_hash {
            chunk(collector.next().unwrap().unwrap(), &file)
        } else {
            first
        };
        collector.chunk(second).unwrap();
        assert_eq!(collector.finish(), Err(Error::Protocol));
    }

    let request = request(0, 3);
    let file = b"abc";
    for mutation in 0..5 {
        let mut collector = Collector::new(request, 11).unwrap();
        let mut bad = header(request, file);
        match mutation {
            0 => bad.id += 1,
            1 => bad.context += 1,
            2 => bad.count = 39,
            3 => bad.version = 0,
            _ => bad.data[32..40].fill(0),
        }
        assert_eq!(collector.open(bad), Err(Error::Protocol));
        assert!(collector.buffered().iter().all(|byte| *byte == 0));
    }
}
