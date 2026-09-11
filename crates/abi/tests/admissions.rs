// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{
    Packet,
    admission::{self as a, AdmissionId, State, Status},
    operation::{Instance, OperationId},
};

#[test]
fn admission_identity_is_canonical_and_separate_from_completion() {
    let id = AdmissionId::new([7; 16], 9).unwrap();
    assert_eq!(id.to_string().parse::<AdmissionId>().unwrap(), id);
    assert!(id.to_string().parse::<OperationId>().is_err());
    for text in [
        id.to_string().to_uppercase(),
        id.to_string() + "x",
        "ad_é".into(),
        "ad_00000000000000000000000000000000_0000000000000001".into(),
    ] {
        assert!(text.parse::<AdmissionId>().is_err());
    }
    for op in [a::GET, a::EXECUTE, a::CANCEL] {
        let p = Packet::decode(&id.packet(op, 3).unwrap().encode()).unwrap();
        assert_eq!(AdmissionId::decode(&p).unwrap(), id);
    }
}

#[test]
fn one_packet_status_rejects_impossible_states_and_noncanonical_data() {
    let mut s = Status {
        id: AdmissionId::new([7; 16], 9).unwrap(),
        state: State::Admitted,
        service_instance: Instance::new([7; 16], 8).unwrap(),
        terminal: 0,
    };
    for (state, terminal) in [
        (State::Admitted, 0),
        (State::Cancelled, 10),
        (State::Committed, 11),
    ] {
        s.state = state;
        s.terminal = terminal;
        let p = s.packet(a::GET, 3).unwrap();
        assert_eq!(
            Status::decode(&Packet::decode(&p.encode()).unwrap()).unwrap(),
            s
        );
        assert_eq!(s.completion().is_some(), state == State::Committed);
        for field in 0..5 {
            let mut bad = p;
            match field {
                0 => bad.id = 1,
                1 => bad.count = 33,
                2 => bad.arg = 4,
                3 => bad.data[32] = 1,
                _ => bad.data[16..24].copy_from_slice(&10u64.to_le_bytes()),
            }
            assert!(Status::decode(&bad).is_err());
        }
    }
    for (state, terminal) in [
        (State::Admitted, 10),
        (State::Cancelled, 0),
        (State::Committed, 9),
    ] {
        s.state = state;
        s.terminal = terminal;
        assert!(s.packet(a::GET, 3).is_err());
    }
}
