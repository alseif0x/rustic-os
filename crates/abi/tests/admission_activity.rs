// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Packet, admission as a, operation::Instance};

#[test]
fn activity_is_not_a_terminal_receipt_and_rejects_reserved_or_inconsistent_fields() {
    let value = a::Activity {
        id: a::AdmissionId::new([7; 16], 9).unwrap(),
        service_instance: Instance::new([7; 16], 8).unwrap(),
        phase: a::ActivityPhase::Stopping,
        cancel_requested: true,
        io_pending: true,
    };
    let p = value.packet(a::REQUEST_CANCEL, 3).unwrap();
    assert_eq!(
        a::Activity::decode(&Packet::decode(&p.encode()).unwrap()),
        Ok(value)
    );
    assert!(a::Status::decode(&p).is_err());
    for mutation in 0..10 {
        let mut bad = p;
        match mutation {
            0 => bad.arg |= 4,
            1 => bad.arg &= !0x100,
            2 => bad.data[24] = 1,
            3 => bad.count = 32,
            4 => bad.op = a::GET,
            5 => bad.data[16..24].copy_from_slice(&10u64.to_le_bytes()),
            6 => bad.id = 1,
            7 => bad.status = 1,
            8 => bad.arg &= !3,
            _ => bad.arg = (bad.arg & !3) | 1,
        }
        assert!(a::Activity::decode(&bad).is_err(), "mutation {mutation}");
    }
}
