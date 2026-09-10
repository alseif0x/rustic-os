// SPDX-License-Identifier: Apache-2.0
use rustic_abi::block::*;
fn read() -> Request {
    Request {
        operation: Operation::Read,
        sector: 0x1234_5678,
        address: 0,
        length: 512,
    }
}
#[test]
fn exact_wire_lengths_and_untrusted_mutations_never_panic() {
    for length in 0..=RESULT_BYTES + 1 {
        let bytes = vec![0xff; length];
        assert!(Request::decode(&bytes).is_err());
        assert!(Geometry::decode(&bytes).is_err());
        assert!(Completion::decode(&bytes).is_err());
    }
    let valid = read().encode();
    for offset in 0..REQUEST_BYTES {
        for value in 0..=255 {
            let mut bytes = valid;
            bytes[offset] = value;
            if let Ok(decoded) = Request::decode(&bytes) {
                assert_eq!(decoded.encode(), bytes);
            }
        }
    }
}
#[test]
fn requests_preserve_endianness_and_reject_ambiguous_fields() {
    let valid = read().encode();
    assert_eq!(&valid[8..12], &[0x78, 0x56, 0x34, 0x12]);
    assert_eq!(Request::decode(&valid), Ok(read()));
    for offset in [4, 5, 6, 7, 28, 29, 30, 31, 16] {
        let mut bytes = valid;
        bytes[offset] = 1;
        assert_eq!(Request::decode(&bytes), Err(Error::Request));
    }
    for length in [0, 1, 511, 513, u32::MAX] {
        assert_eq!(
            Request::decode(&Request { length, ..read() }.encode()),
            Err(Error::Size)
        );
    }
    for operation in [Operation::Read, Operation::Write, Operation::Flush] {
        let request = Request {
            operation,
            sector: 0,
            address: 0,
            length: if operation == Operation::Flush {
                0
            } else {
                512
            },
        };
        assert_eq!(Request::decode(&request.encode()), Ok(request));
    }
    let flush = Request {
        operation: Operation::Flush,
        sector: 0,
        address: 0,
        length: 0,
    };
    for offset in [8, 16, 24] {
        let mut bytes = flush.encode();
        bytes[offset] = 1;
        assert_eq!(Request::decode(&bytes), Err(Error::Request));
    }
}
#[test]
fn completion_effects_and_payloads_are_checked_together() {
    for operation in [Operation::Read, Operation::Write, Operation::Flush] {
        for status in [
            Status::Success,
            Status::Cancelled,
            Status::Io,
            Status::Timeout,
            Status::Protocol,
            Status::Unavailable,
        ] {
            for effect in [Effect::None, Effect::Completed, Effect::Unknown] {
                let valid = if operation == Operation::Read
                    || matches!(status, Status::Cancelled | Status::Unavailable)
                {
                    effect == Effect::None
                } else if status == Status::Success {
                    effect == Effect::Completed
                } else {
                    effect == Effect::Unknown
                };
                let completion = Completion {
                    id: 17,
                    operation,
                    status,
                    effect,
                    data: [0x53; SECTOR],
                };
                let bytes = completion.encode();
                assert_eq!(Completion::decode(&bytes).is_ok(), valid);
                if valid {
                    let decoded = Completion::decode(&bytes).unwrap();
                    assert_eq!(
                        decoded.data,
                        if operation == Operation::Read && status == Status::Success {
                            [0x53; SECTOR]
                        } else {
                            [0; SECTOR]
                        }
                    );
                }
            }
        }
    }
    let completion = Completion {
        id: 1,
        operation: Operation::Write,
        status: Status::Success,
        effect: Effect::Completed,
        data: [0; SECTOR],
    };
    for (offset, value) in [
        (8, 0),
        (15, 255),
        (4, 99),
        (20, 99),
        (16, 1),
        (24, 1),
        (32, 1),
        (543, 1),
    ] {
        let mut bytes = completion.encode();
        bytes[offset] = value;
        assert_eq!(Completion::decode(&bytes), Err(Error::Protocol));
    }
}
#[test]
fn geometry_rejects_unknown_rights_and_impossible_limits() {
    let geometry = Geometry {
        sectors: 8_388_608,
        rights: ALL,
        read_only: false,
    };
    assert_eq!(Geometry::decode(&geometry.encode()), Ok(geometry));
    for (offset, value) in [
        (0, 2),
        (2, 31),
        (4, 0),
        (4, 8),
        (10, 0),
        (17, 0),
        (21, 0),
        (24, 2),
        (28, 1),
    ] {
        let mut bytes = geometry.encode();
        bytes[offset] = value;
        assert!(Geometry::decode(&bytes).is_err(), "offset={offset}");
    }
    for error in [
        Error::Handle,
        Error::Denied,
        Error::Address,
        Error::Size,
        Error::Version,
        Error::Request,
        Error::Busy,
        Error::Quota,
        Error::Range,
        Error::ReadOnly,
        Error::Unavailable,
        Error::NoRequest,
        Error::WouldBlock,
        Error::Protocol,
    ] {
        assert_eq!(Error::decode(error.code()), Err(error));
    }
    assert_eq!(Error::decode(MAX_ID), Ok(MAX_ID));
    assert_eq!(Error::decode(MAX_ID + 1), Err(Error::Protocol));
}
