// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, Packet, admission::*, operation::Instance};

fn retained() -> Status {
    Status {
        id: AdmissionId::new([7; 16], 9).unwrap(),
        service_instance: Instance::new([7; 16], 8).unwrap(),
        state: State::Admitted,
        terminal: 0,
    }
}

#[test]
fn stable_identity_distinguishes_live_progress_from_confirmed_records() {
    let old = retained();
    let request = old.id.packet(OBSERVE, 11).unwrap();
    assert_eq!(request.arg, OBSERVATION_VERSION);
    assert_eq!(
        AdmissionId::decode(&Packet::decode(&request.encode()).unwrap()),
        Ok(old.id)
    );
    let mut views = vec![Observation::Retained(old)];
    for phase in [
        ActivityPhase::Queued,
        ActivityPhase::Running,
        ActivityPhase::Stopping,
        ActivityPhase::Settling,
    ] {
        views.push(Observation::Active(Activity {
            id: old.id,
            service_instance: old.service_instance,
            phase,
            cancel_requested: phase == ActivityPhase::Stopping,
            io_pending: phase != ActivityPhase::Queued,
        }));
    }
    for state in [State::Cancelled, State::Committed] {
        views.push(Observation::Retained(Status {
            state,
            terminal: 12,
            ..old
        }));
    }
    for view in views {
        let packet = Packet::decode(&view.packet(11).unwrap().encode()).unwrap();
        assert_eq!(Observation::decode(&packet), Ok(view));
        assert_eq!(view.id(), old.id);
        assert_eq!(view.service_instance(), old.service_instance);
        // A wrapper cannot be consumed as an old reply with different semantics.
        assert_eq!(Status::decode(&packet), Err(Error::Protocol));
        assert_eq!(Activity::decode(&packet), Err(Error::Protocol));
    }
}

#[test]
fn profile_padding_and_terminal_invariants_are_checked() {
    let view = Observation::Retained(retained());
    let good = view.packet(11).unwrap();
    for fault in 0..8 {
        let mut p = good;
        match fault {
            0 => p.id = 2,
            1 => p.op = GET,
            2 => p.count = 31,
            3 => p.status = Error::Denied as u8,
            4 => p.data[39] = 1,
            5 => p.arg = 3, // A committed record must carry a later terminal sequence.
            6 => p.data[24] = 12, // Prepared cannot carry a terminal sequence.
            _ => p.version = 0,
        }
        assert_eq!(
            Observation::decode(&p),
            Err(Error::Protocol),
            "fault {fault}"
        );
    }
    let mut p = good;
    p.arg = 1 | 0x100;
    p.count = 24;
    assert_eq!(
        Observation::decode(&p),
        Err(Error::Protocol),
        "running cannot already have a stop latch"
    );
}
