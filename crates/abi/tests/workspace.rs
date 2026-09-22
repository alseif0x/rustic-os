// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{operation as legacy, reference::*, workspace::*, *};

fn old_operation() -> legacy::Operation {
    let workspace = Workspace::new([7; 16], 4).unwrap();
    legacy::Operation {
        id: legacy::OperationId::new([7; 16], 10).unwrap(),
        service_instance: legacy::Instance::new([7; 16], 8).unwrap(),
        workspace,
        resource: Resource::new(workspace, 257).unwrap(),
        previous_version: Version::new(5).unwrap(),
        version: Version::new(10).unwrap(),
        size: 100,
        retry: legacy::Retry {
            epoch: Epoch::new(1).unwrap(),
            key: legacy::Key::new(42).unwrap(),
        },
        sha256: [0xaa; 32],
    }
}
fn operation(size: u32) -> Operation {
    let old = old_operation();
    Operation {
        id: old.id,
        service_instance: old.service_instance,
        workspace: old.workspace,
        resource: old.resource,
        previous_version: old.previous_version,
        version: old.version,
        size,
        retry: old.retry,
        sha256: old.sha256,
    }
}
fn replacement() -> Replacement {
    let old = old_operation();
    Replacement {
        request: legacy::Replacement {
            workspace: old.workspace,
            resource: old.resource,
            expected_version: old.previous_version,
            retry: old.retry,
        },
    }
}

#[test]
fn large_requests_round_trip_and_profiles_never_fall_back() {
    let request = replacement();
    for size in [0, 100, 1024, 1025, 65535, 65536, MAX_FILE_BYTES as usize] {
        let packet = request.packet(size, 9).unwrap();
        let wire = packet.encode();
        assert_eq!(wire.len(), 64);
        assert_eq!(&wire[60..64], &[2, 0, 0, 0]);
        let decoded = Packet::decode(&wire).unwrap();
        assert_eq!(decoded.arg as usize, size);
        assert_eq!(
            Replacement::decode(&decoded).unwrap().request,
            request.request
        );
        assert!(legacy::Replacement::decode(&decoded).is_err());
    }
    let old = request.request.packet(100, 9).unwrap();
    assert!(Replacement::decode(&old).is_err());
    assert_eq!(
        request.packet(MAX_FILE_BYTES as usize + 1, 9),
        Err(Error::Size)
    );
    assert_eq!(request.packet(usize::MAX, 9), Err(Error::Size));
    let mut bad = request.packet(100, 9).unwrap();
    bad.data[36] = 3;
    assert!(Replacement::decode(&bad).is_err());
    bad = request.packet(100, 9).unwrap();
    bad.arg = MAX_FILE_BYTES + 1;
    assert!(Replacement::decode(&bad).is_err());
    bad = request.packet(100, 9).unwrap();
    bad.status = Error::Denied as u8;
    assert!(Replacement::decode(&bad).is_err());
}

#[test]
fn receipt_size_and_profile_have_fixed_offsets_and_bounded_fragments() {
    for size in [0, 100, 1024, 1025, 65535, 65536, MAX_FILE_BYTES] {
        let receipt = operation(size);
        let bytes = receipt.encode().unwrap();
        assert_eq!(&bytes[64..68], &size.to_le_bytes());
        assert_eq!(&bytes[68..72], &[2, 0, 0, 0]);
        assert_eq!(Operation::decode(&bytes), Ok(receipt));
        assert!(legacy::Operation::decode(&bytes).is_err());
        let mut collected = [0; RECEIPT_BYTES];
        for offset in [0, 40, 80] {
            let packet = receipt.part(OPERATION_PART, 9, offset).unwrap();
            let packet = Packet::decode(&packet.encode())
                .unwrap()
                .checked_reply(OPERATION_PART, 9)
                .unwrap();
            assert_eq!(packet.id as usize, offset);
            assert_eq!(packet.arg as usize, RECEIPT_BYTES);
            assert_eq!(packet.version, receipt.id.sequence());
            collected[offset..offset + packet.count as usize].copy_from_slice(packet.payload());
        }
        assert_eq!(Operation::decode(&collected), Ok(receipt));
    }
    let old = old_operation().encode().unwrap();
    assert_eq!(&old[64..72], &[100, 0, 0, 0, 0, 0, 0, 0]);
    assert!(Operation::decode(&old).is_err());
    assert!(operation(MAX_FILE_BYTES + 1).encode().is_err());
    for offset in [1, 39, 41, 104, usize::MAX] {
        assert_eq!(
            operation(100).part(OPERATION_PART, 9, offset),
            Err(Error::Offset)
        );
    }
}

#[test]
fn wide_receipts_preserve_identity_version_and_digest_binding() {
    let receipt = operation(200_000);
    let mut bytes = receipt.encode().unwrap();
    bytes[68] = 3;
    assert!(Operation::decode(&bytes).is_err());
    bytes = receipt.encode().unwrap();
    bytes[64..68].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Operation::decode(&bytes).is_err());
    let mut wrong = receipt;
    wrong.workspace = Workspace::new([8; 16], 4).unwrap();
    assert!(wrong.encode().is_err());
    wrong = receipt;
    wrong.previous_version = wrong.version;
    assert!(wrong.encode().is_err());
    wrong = receipt;
    wrong.service_instance = legacy::Instance::new([7; 16], 11).unwrap();
    assert!(wrong.encode().is_err());
    assert_eq!(&receipt.encode().unwrap()[72..], &[0xaa; 32]);
}

#[test]
fn historical_lookups_select_the_profile_without_changing_identity() {
    let old = old_operation();
    for query in [
        legacy::Lookup::Retry {
            workspace: old.workspace,
            retry: old.retry,
        },
        legacy::Lookup::Id(old.id),
    ] {
        let lookup = Lookup { query };
        let packet = lookup.packet(9);
        let decoded = Packet::decode(&packet.encode()).unwrap();
        let restored = Lookup::decode(&decoded).unwrap().packet(9);
        assert_eq!(restored, packet);
        assert!(legacy::Lookup::decode(&packet).is_err());
        assert!(Lookup::decode(&query.packet(9)).is_err());
        let mut bad = packet;
        bad.data[39] = 1;
        assert!(Lookup::decode(&bad).is_err());
    }
    for offset in [0, 40, 80] {
        let mut p = Lookup {
            query: legacy::Lookup::Id(old.id),
        }
        .packet(9);
        p.op = OPERATION_PART;
        p.arg = offset;
        assert!(Lookup::decode(&p).is_ok());
        p.arg = 104;
        assert!(Lookup::decode(&p).is_err());
    }
}
