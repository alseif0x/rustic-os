// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{operation::*, reference::*, *};
fn operation() -> Operation {
    let workspace = Workspace::new([7; 16], 4).unwrap();
    Operation {
        id: OperationId::new([7; 16], 10).unwrap(),
        service_instance: Instance::new([7; 16], 8).unwrap(),
        workspace,
        resource: Resource::new(workspace, 6).unwrap(),
        previous_version: Version::new(5).unwrap(),
        version: Version::new(10).unwrap(),
        size: 1024,
        retry: Retry {
            epoch: Epoch::new(1).unwrap(),
            key: Key::new(42).unwrap(),
        },
        sha256: [0xaa; 32],
    }
}
#[test]
fn canonical_identifiers_and_receipt_fragments_are_lossless_and_bounded() {
    let op = operation();
    assert_eq!(op.id.to_string().parse::<OperationId>().unwrap(), op.id);
    assert_eq!(
        op.retry.key.to_string().parse::<Key>().unwrap(),
        op.retry.key
    );
    assert!("k_0000000000000000".parse::<Key>().is_err());
    for bad in [
        "k_00000000000000AA",
        "k_0000000000000001\n",
        "k_é000000000000001",
    ] {
        assert!(bad.parse::<Key>().is_err());
    }
    let bytes = op.encode().unwrap();
    assert_eq!(Operation::decode(&bytes).unwrap(), op);
    let mut assembled = [0; RECEIPT_BYTES];
    for offset in [0, 40, 80] {
        let p = op.part(OPERATION_PART, 9, offset).unwrap();
        let p = Packet::decode(&p.encode())
            .unwrap()
            .checked_reply(OPERATION_PART, 9)
            .unwrap();
        assembled[offset..offset + p.count as usize].copy_from_slice(p.payload());
    }
    assert_eq!(Operation::decode(&assembled).unwrap(), op);
    for offset in [1, 39, 41, 104, usize::MAX] {
        assert_eq!(op.part(OPERATION_PART, 9, offset), Err(Error::Offset));
    }
    for index in [66, 67, 68, 69, 70, 71] {
        let mut b = bytes;
        b[index] = 1;
        assert_eq!(Operation::decode(&b), Err(Error::Protocol));
    }
    let mut bad = op;
    bad.service_instance = Instance::new([8; 16], 8).unwrap();
    assert_eq!(bad.encode(), Err(Error::Protocol));
    bad = op;
    bad.previous_version = bad.version;
    assert_eq!(bad.encode(), Err(Error::Protocol));
}
#[test]
fn replacement_checks_workspace_and_query_needs_no_resource() {
    let op = operation();
    let mut request = Replacement {
        workspace: op.workspace,
        resource: op.resource,
        expected_version: op.previous_version,
        retry: op.retry,
    };
    let p = request.packet(1024, 9).unwrap();
    assert_eq!(Replacement::decode(&p).unwrap(), request);
    assert_eq!(request.packet(1025, 9), Err(Error::Size));
    request.workspace = Workspace::new([8; 16], 4).unwrap();
    assert_eq!(request.packet(0, 9), Err(Error::Denied));
    let lookup = Lookup::Retry {
        workspace: op.workspace,
        retry: op.retry,
    };
    let p = lookup.packet(9);
    assert_eq!(p.id, 4);
    assert!(matches!(Lookup::decode(&p).unwrap(), Lookup::Retry { .. }));
}
