// SPDX-License-Identifier: Apache-2.0
//! The production SDK/RPC over a safe host transport, without native syscall claims.
pub use rustic_sdk::{Error, abi, rpc::state};
#[allow(dead_code, unused_imports)]
#[path = "../src/files/mod.rs"]
mod files;
#[path = "../src/rpc/progress.rs"]
mod progress;
pub use progress::{Blocking, Progress};
#[allow(dead_code)]
#[path = "../src/rpc/client.rs"]
mod rpc_client;
mod rpc {
    pub use super::{Blocking, Progress, rpc_client::Rpc};
}
#[path = "support/file_transport.rs"]
mod transport;
use rustic_abi::files::{
    Error as E, Packet,
    admission::{self as a, AdmissionId, State, Status},
    operation::{Instance, Key, Replacement, Retry},
    reference::{Epoch, Resource, Version, Workspace},
};
pub use transport::{ipc, runtime};

fn fixture() -> (Replacement, Status) {
    let workspace = Workspace::new([7; 16], 4).unwrap();
    (
        Replacement {
            workspace,
            resource: Resource::new(workspace, 6).unwrap(),
            expected_version: Version::new(2).unwrap(),
            retry: Retry {
                epoch: Epoch::new(1).unwrap(),
                key: Key::new(42).unwrap(),
            },
        },
        Status {
            id: AdmissionId::new([7; 16], 9).unwrap(),
            state: State::Admitted,
            service_instance: Instance::new([7; 16], 9).unwrap(),
            terminal: 0,
        },
    )
}

#[test]
fn observation_uses_one_read_exchange_and_rejects_wrong_identity_or_profile() {
    let (_, status) = fixture();
    for fault in 0..5 {
        transport::reset();
        transport::respond(move |request| {
            assert_eq!(request.op, a::OBSERVE);
            assert_eq!(request.arg, a::OBSERVATION_VERSION);
            let mut p = a::Observation::Retained(status)
                .packet(request.context)
                .unwrap();
            match fault {
                1 => p.version += 1,
                2 => p.id += 1,
                3 => p.data[24] = 1,
                4 => {
                    p = Packet::new(a::OBSERVE);
                    p.context = request.context;
                    p.status = E::UnsupportedVersion as u8;
                }
                _ => (),
            }
            p
        });
        let mut client = files::Client::new(1, 7, 11);
        let result = client.admission_observe(status.id);
        assert_eq!(
            result,
            match fault {
                0 => Ok(a::Observation::Retained(status)),
                4 => Err(E::UnsupportedVersion),
                _ => Err(E::Protocol),
            }
        );
        assert_eq!(
            transport::requests().len(),
            1,
            "observation must not query twice, resume or retry"
        );
    }
}

#[test]
fn scheduling_returns_queued_and_never_retries_a_lost_or_misbound_reply() {
    let (_, status) = fixture();
    for fault in 0..5 {
        transport::reset();
        transport::respond(move |p| {
            let mut view = a::Activity {
                id: status.id,
                service_instance: status.service_instance,
                phase: a::ActivityPhase::Queued,
                cancel_requested: false,
                io_pending: false,
            };
            if fault == 1 {
                view.id = AdmissionId::new([7; 16], 10).unwrap();
            }
            let mut packet = view.packet(p.op, p.context).unwrap();
            if fault == 2 {
                packet.context += 1;
            }
            if fault == 3 {
                packet.arg |= 0x200;
            }
            packet
        });
        transport::wrong_peer(fault == 4);
        let mut client = files::Client::new(1, 7, 11);
        let result = client.admission_schedule(status.id);
        if fault == 0 {
            assert_eq!(result.unwrap().phase, a::ActivityPhase::Queued);
        } else {
            assert_eq!(result, Err(E::Uncertain));
        }
        assert_eq!(transport::requests().len(), 1);
    }
}

#[test]
fn active_control_rejects_corrupt_or_unbound_results_without_replaying_a_stop() {
    let (_, status) = fixture();
    for op in [a::ACTIVITY, a::REQUEST_CANCEL] {
        for fault in 0..6 {
            transport::reset();
            transport::respond(move |p| {
                let mut activity = a::Activity {
                    id: status.id,
                    service_instance: status.service_instance,
                    phase: a::ActivityPhase::Settling,
                    cancel_requested: p.op == a::REQUEST_CANCEL,
                    io_pending: true,
                };
                if fault == 1 {
                    activity.id = AdmissionId::new([8; 16], 9).unwrap();
                    activity.service_instance = Instance::new([8; 16], 9).unwrap();
                }
                let mut reply = activity.packet(p.op, p.context).unwrap();
                match fault {
                    2 => reply.arg &= !0x100,
                    3 => reply.count = 32,
                    4 => reply.context += 1,
                    _ => (),
                }
                reply
            });
            transport::wrong_peer(fault == 5);
            let mut client = files::Client::new(1, 7, 11);
            let result = if op == a::ACTIVITY {
                client.admission_activity(status.id)
            } else {
                client.admission_request_cancel(status.id)
            };
            if fault == 0 || fault == 2 && op == a::ACTIVITY {
                assert!(result.is_ok());
            } else {
                assert_eq!(
                    result,
                    Err(if op == a::ACTIVITY {
                        E::Protocol
                    } else {
                        E::Uncertain
                    })
                );
            }
            assert_eq!(transport::requests().len(), 1);
        }
    }
}
fn respond(mut status: Status, corrupt: bool) {
    transport::reset();
    transport::respond(move |p| {
        if matches!(p.op, a::OPEN | a::CHUNK | a::ABORT) {
            let mut r = Packet::new(p.op);
            r.context = p.context;
            return r;
        }
        if p.op == a::EXECUTE {
            status.state = State::Committed;
            status.terminal = 10;
        }
        if p.op == a::CANCEL && status.state == State::Admitted {
            status.state = State::Cancelled;
            status.terminal = 10;
        }
        let mut r = status.packet(p.op, p.context).unwrap();
        if corrupt {
            r.arg = 99;
        }
        r
    });
}
#[test]
fn sdk_admits_without_execution_then_explicitly_executes_or_cancels() {
    let (r, accepted) = fixture();
    for cancel in [false, true] {
        respond(accepted, false);
        let mut client = files::Client::new(1, 7, 11);
        assert_eq!(client.admit_file(r, &[b'x'; 81]), Ok(accepted));
        assert_eq!(client.admission_retry(r.workspace, r.retry), Ok(accepted));
        assert_eq!(client.admission_get(accepted.id), Ok(accepted));
        assert!(
            transport::requests()
                .iter()
                .all(|(_, _, p)| !matches!(p.op, a::EXECUTE | a::CANCEL))
        );
        let result = if cancel {
            client.admission_cancel(accepted.id)
        } else {
            client.admission_execute(accepted.id)
        }
        .unwrap();
        assert_eq!(
            result.state,
            if cancel {
                State::Cancelled
            } else {
                State::Committed
            }
        );
        assert_eq!(
            transport::requests()
                .iter()
                .filter(|(_, _, p)| p.op == a::ACCEPT)
                .count(),
            1
        );
    }
}
#[test]
fn malformed_durable_reply_is_uncertain_and_never_replayed() {
    let (r, accepted) = fixture();
    for op in [a::ACCEPT, a::GET, a::CANCEL, a::EXECUTE] {
        respond(accepted, true);
        let mut client = files::Client::new(1, 7, 11);
        let result = match op {
            a::ACCEPT => client.admit_file(r, b"x"),
            a::GET => client.admission_get(accepted.id),
            a::CANCEL => client.admission_cancel(accepted.id),
            _ => client.admission_execute(accepted.id),
        };
        assert_eq!(
            result,
            Err(if op == a::GET {
                E::Protocol
            } else {
                E::Uncertain
            })
        );
        assert_eq!(
            transport::requests()
                .iter()
                .filter(|(_, _, p)| p.op == op)
                .count(),
            1
        );
    }
}
#[test]
fn transport_loss_after_submission_is_uncertain_and_unbound_calls_do_not_send() {
    let (r, status) = fixture();
    for op in [a::CANCEL, a::EXECUTE] {
        respond(status, false);
        transport::wrong_peer(true);
        let mut client = files::Client::new(1, 7, 11);
        assert_eq!(
            if op == a::CANCEL {
                client.admission_cancel(status.id)
            } else {
                client.admission_execute(status.id)
            },
            Err(E::Uncertain)
        );
        assert_eq!(transport::requests().len(), 1);
    }
    for op in [a::ACCEPT, a::EXECUTE, a::CANCEL] {
        for asynchronous in [false, true] {
            respond(status, false);
            transport::wrong_peer(true);
            let mut client = files::Client::new(1, 7, 11);
            let packet = if op == a::ACCEPT {
                Packet::new(op)
            } else {
                status.id.packet(op, 11).unwrap()
            };
            if asynchronous {
                client.submit(packet).unwrap();
                assert_eq!(client.poll(), Err(E::Uncertain));
            } else {
                assert_eq!(client.request(packet), Err(E::Uncertain));
            }
            assert_eq!(transport::requests().len(), 1);
        }
    }
    transport::reset();
    let mut client = files::Client::new(0, 0, 0);
    assert_eq!(client.admit_file(r, b"x"), Err(E::Unavailable));
    assert_eq!(client.admission_cancel(status.id), Err(E::Unavailable));
    assert_eq!(client.admission_get(status.id), Err(E::Unavailable));
    assert_eq!(transport::counts(), (0, 0, 0));
}
