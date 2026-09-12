// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, Packet, admission::*, operation::Instance};

fn status(state: State) -> Status {
    Status {
        id: AdmissionId::new([7; 16], 9).unwrap(),
        service_instance: Instance::new([7; 16], 8).unwrap(),
        state,
        terminal: if state == State::Admitted { 0 } else { 12 },
    }
}
#[test]
fn explicit_profiles_preserve_states_and_all_distinct_causes() {
    for state in [State::Admitted, State::Cancelled, State::Committed] {
        for prevention in [
            None,
            Some(PreventionReason::Unknown),
            Some(PreventionReason::Requested),
            Some(PreventionReason::VersionConflict),
            Some(PreventionReason::AuthorityLost),
        ] {
            let view = ObservationV2::Retained {
                status: status(state),
                prevention,
            };
            if (state == State::Cancelled) != prevention.is_some() {
                assert_eq!(view.packet(11), Err(Error::Protocol));
                continue;
            }
            let p = Packet::decode(&view.packet(11).unwrap().encode()).unwrap();
            assert_eq!(ObservationV2::decode(&p), Ok(view));
            assert_eq!(Observation::decode(&p), Err(Error::Protocol));
            assert_eq!(
                ObservationV2::decode(&view.coarse().packet(11).unwrap()),
                Err(Error::Protocol)
            );
            assert_eq!(p.data[32], prevention.map_or(0, |r| r as u8));
        }
    }
    let id = status(State::Admitted).id;
    let request = ObservationV2::request(id, 11).unwrap();
    assert_eq!(request.arg, 2);
    assert_eq!(AdmissionId::decode(&request), Ok(id));
    assert_eq!(id.packet(OBSERVE, 11).unwrap().arg, 1);
    for bad in [0, 3, u32::MAX] {
        let mut p = request;
        p.arg = bad;
        assert_eq!(AdmissionId::decode(&p), Err(Error::Protocol));
    }
}
#[test]
fn live_profile_carries_no_speculative_cause() {
    for phase in [
        ActivityPhase::Queued,
        ActivityPhase::Running,
        ActivityPhase::Stopping,
        ActivityPhase::Settling,
    ] {
        let s = status(State::Admitted);
        let view = ObservationV2::Active(Activity {
            id: s.id,
            service_instance: s.service_instance,
            phase,
            cancel_requested: phase == ActivityPhase::Stopping,
            io_pending: phase != ActivityPhase::Queued,
        });
        let mut p = view.packet(11).unwrap();
        assert_eq!(p.count, 24);
        assert_eq!(ObservationV2::decode(&p), Ok(view));
        p.data[32] = PreventionReason::Requested as u8;
        assert_eq!(ObservationV2::decode(&p), Err(Error::Protocol));
    }
}
#[test]
fn malformed_or_contradictory_retained_facts_are_rejected() {
    let good = ObservationV2::Retained {
        status: status(State::Cancelled),
        prevention: Some(PreventionReason::Requested),
    }
    .packet(11)
    .unwrap();
    for fault in 0..10 {
        let mut p = good;
        match fault {
            0 => p.id = 1,
            1 => p.data[32] = 0,
            2 => p.data[32] = 5,
            3 => p.data[33] = 1,
            4 => p.data[39] = 1,
            5 => p.count = 32,
            6 => p.arg = 3,
            7 => p.data[24] = 0,
            8 => p.status = 17,
            _ => p.op = GET,
        }
        assert_eq!(
            ObservationV2::decode(&p),
            Err(Error::Protocol),
            "fault {fault}"
        );
    }
}
