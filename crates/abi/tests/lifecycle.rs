// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, Packet, admission as a, lifecycle as l, operation::Instance};

fn status(state: a::State) -> a::Status {
    a::Status {
        id: a::AdmissionId::new([7; 16], 9).unwrap(),
        service_instance: Instance::new([7; 16], 8).unwrap(),
        state,
        terminal: if state == a::State::Admitted { 0 } else { 12 },
    }
}

#[test]
fn retained_projection_keeps_identity_without_inventing_stop_history() {
    for (retained, prevention, expected) in [
        (a::State::Admitted, None, l::State::Prepared),
        (
            a::State::Committed,
            None,
            l::State::Succeeded {
                completion_id: status(a::State::Committed).completion().unwrap(),
            },
        ),
        (
            a::State::Cancelled,
            Some(a::PreventionReason::Unknown),
            l::State::Prevented,
        ),
        (
            a::State::Cancelled,
            Some(a::PreventionReason::Requested),
            l::State::Cancelled,
        ),
        (
            a::State::Cancelled,
            Some(a::PreventionReason::VersionConflict),
            l::State::Failed {
                failure: l::Failure::VersionConflict,
            },
        ),
        (
            a::State::Cancelled,
            Some(a::PreventionReason::AuthorityLost),
            l::State::Failed {
                failure: l::Failure::AccessDenied,
            },
        ),
    ] {
        let old = status(retained);
        let result = l::Operation::try_from(a::ObservationV2::Retained {
            status: old,
            prevention,
        })
        .unwrap();
        assert_eq!(result.id, old.id);
        assert_eq!(result.service_instance, old.service_instance);
        assert_eq!(result.state, expected);
    }
    assert!(
        l::Operation::try_from(a::ObservationV2::Retained {
            status: status(a::State::Cancelled),
            prevention: None
        })
        .is_err()
    );
}

#[test]
fn live_projection_distinguishes_uncertain_effect_from_a_stop_latch() {
    for (phase, stop, expected) in [
        (
            a::ActivityPhase::Queued,
            false,
            l::State::Queued {
                stop_pending: false,
            },
        ),
        (
            a::ActivityPhase::Running,
            false,
            l::State::Running {
                stop_pending: false,
            },
        ),
        (
            a::ActivityPhase::Stopping,
            true,
            l::State::Running { stop_pending: true },
        ),
        (
            a::ActivityPhase::Settling,
            false,
            l::State::Reconciling {
                stop_pending: false,
            },
        ),
        (
            a::ActivityPhase::Settling,
            true,
            l::State::Reconciling { stop_pending: true },
        ),
    ] {
        let v = a::ObservationV2::Active(a::Activity {
            id: status(a::State::Admitted).id,
            service_instance: status(a::State::Admitted).service_instance,
            phase,
            cancel_requested: stop,
            io_pending: false,
        });
        assert_eq!(l::Operation::try_from(v).unwrap().state, expected);
    }
}

#[test]
fn cancellation_codec_is_strict_and_cannot_carry_inspection_fields() {
    let id = status(a::State::Admitted).id;
    let request = l::CancelAck::request(id, 7).unwrap();
    assert_eq!(Packet::decode(&request.encode()).unwrap(), request);
    assert_eq!(l::CancelAck::decode_request(&request), Ok(id));
    for disposition in [
        l::Disposition::Requested,
        l::Disposition::AlreadyRequested,
        l::Disposition::TooLate,
    ] {
        let ack = l::CancelAck { id, disposition };
        let packet = ack.packet(7).unwrap();
        assert_eq!(
            l::CancelAck::decode(&Packet::decode(&packet.encode()).unwrap()),
            Ok(ack)
        );
        assert!(a::ObservationV2::decode(&packet).is_err());
        for fault in 0..9 {
            let mut p = packet;
            match fault {
                0 => p.op = a::REQUEST_CANCEL,
                1 => p.status = Error::Denied as u8,
                2 => p.id = 1,
                3 => p.arg = 0,
                4 => p.arg = 4,
                5 => p.count = 24,
                6 => p.data[16] = 1,
                7 => p.data[..16].fill(0),
                _ => p.version = 0,
            }
            assert!(l::CancelAck::decode(&p).is_err(), "fault={fault}");
        }
    }
}
