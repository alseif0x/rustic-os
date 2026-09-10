// SPDX-License-Identifier: Apache-2.0
//! Pure client collection tests. Native peer/transport execution is tested in QEMU.
#[path = "../src/files/range/collector.rs"]
mod collector;
use collector::Collector;
use rustic_abi::files::{
    Error, Packet, READ_CHUNK,
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
    let start = request.offset as usize;
    let end = file.len().min(start + usize::from(request.length));
    Header {
        id: 6,
        size: file.len() as u64,
        version: Version::new(7).unwrap(),
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

#[test]
fn complete_binary_and_partial_ranges_require_all_chunks_and_the_range_hash() {
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
        let mut output = [0xa5; 1024];
        let mut collector = Collector::new(request, 11, &mut output).unwrap();
        collector.open(header(request, &file)).unwrap();
        let mut next_offset = offset;
        while let Some(p) = collector.next().unwrap() {
            let decoded = Request::decode(&p).unwrap();
            assert_eq!(decoded.offset, next_offset);
            next_offset += u64::from(decoded.length);
            collector.chunk(chunk(p, &file)).unwrap();
        }
        let info = collector.finish().unwrap();
        let end = file.len().min(offset as usize + usize::from(length));
        assert_eq!(&output[..info.length], &file[offset as usize..end]);
        assert!(output[info.length..].iter().all(|b| *b == 0));
        assert_eq!(info.eof, end == file.len());
    }
}

#[test]
fn empty_and_eof_reads_validate_the_empty_hash_without_a_data_packet() {
    for file in [b"".as_slice(), b"abc".as_slice()] {
        let request = request(file.len() as u64, 1);
        let mut output = [0xa5; 1];
        let mut collector = Collector::new(request, 11, &mut output).unwrap();
        collector.open(header(request, file)).unwrap();
        assert_eq!(collector.next().unwrap(), None);
        let info = collector.finish().unwrap();
        assert_eq!((info.length, info.eof), (0, true));
        assert_eq!(output, [0]);
    }
}

#[test]
fn missing_or_abandoned_chunks_clear_previously_collected_bytes() {
    for abandon in [false, true] {
        let file = [0x5a; 80];
        let request = request(0, 80);
        let mut output = [0xa5; 80];
        let mut collector = Collector::new(request, 11, &mut output).unwrap();
        collector.open(header(request, &file)).unwrap();
        collector
            .chunk(chunk(collector.next().unwrap().unwrap(), &file))
            .unwrap();
        if abandon {
            drop(collector);
        } else {
            assert_eq!(collector.finish(), Err(Error::Protocol));
        }
        assert_eq!(output, [0; 80]);
    }
}

#[test]
fn changed_version_or_malformed_chunk_fails_the_whole_read() {
    let file: Vec<u8> = (0..80).collect();
    for mutation in 0..7 {
        let request = request(0, 80);
        let mut output = [0xa5; 80];
        let mut collector = Collector::new(request, 11, &mut output).unwrap();
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
            _ => bad.op = 6,
        }
        assert_eq!(collector.chunk(bad), Err(Error::Protocol));
        assert_eq!(collector.finish(), Err(Error::Protocol));
        assert_eq!(output, [0; 80]);
    }
}

#[test]
fn corrupt_hash_or_repeated_data_cannot_become_a_successful_range() {
    let file: Vec<u8> = (0..80).collect();
    for corrupt_hash in [false, true] {
        let request = request(0, 80);
        let mut output = [0xa5; 80];
        let mut collector = Collector::new(request, 11, &mut output).unwrap();
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
        assert_eq!(output, [0; 80]);
    }
}

#[test]
fn malformed_header_padding_and_undersized_output_are_rejected_and_cleared() {
    let request = request(0, 3);
    let file = b"abc";
    for mutation in 0..5 {
        let mut output = [0xa5; 3];
        let mut collector = Collector::new(request, 11, &mut output).unwrap();
        let mut bad = header(request, file);
        match mutation {
            0 => bad.id += 1,
            1 => bad.context += 1,
            2 => bad.count = 39,
            3 => bad.version = 0,
            _ => bad.data[32..].fill(0),
        }
        assert_eq!(collector.open(bad), Err(Error::Protocol));
        assert_eq!(collector.finish(), Err(Error::Protocol));
        assert_eq!(output, [0; 3]);
    }
    let mut output = [0xa5; 3];
    let mut collector = Collector::new(request, 11, &mut output).unwrap();
    collector.open(header(request, file)).unwrap();
    let mut bad = chunk(collector.next().unwrap().unwrap(), file);
    bad.data[3] = 1;
    assert_eq!(collector.chunk(bad), Err(Error::Protocol));
    drop(collector);
    assert_eq!(output, [0; 3]);
    let mut small = [0xa5; 2];
    assert!(matches!(
        Collector::new(request, 11, &mut small),
        Err(Error::Size)
    ));
    assert_eq!(small, [0; 2]);
}
