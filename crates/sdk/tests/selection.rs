// SPDX-License-Identifier: Apache-2.0
//! Production SDK/RPC behavior over a safe host IPC fixture; no guest claims.
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
use rustic_abi::{
    files::{
        Error as E, Packet, admission as a, lifecycle as l, negotiation as n, operation::Instance,
    },
    services::{Availability, Method},
};
pub use transport::{ipc, runtime};

fn id() -> a::AdmissionId {
    a::AdmissionId::new([7; 16], 9).unwrap()
}

fn reply(p: Packet) -> Packet {
    match p.op {
        n::DESCRIBE => n::Descriptor::reviewed(
            n::decode_request(&p).unwrap(),
            Availability::Available,
            n::Limits {
                retained_operations: 2,
                execution_tickets: 2,
                active_publications: 1,
            },
        )
        .unwrap()
        .packet(p.context)
        .unwrap(),
        a::OBSERVE => a::ObservationV2::Retained {
            status: a::Status {
                id: id(),
                state: a::State::Admitted,
                service_instance: Instance::new([7; 16], 9).unwrap(),
                terminal: 0,
            },
            prevention: None,
        }
        .packet(p.context)
        .unwrap(),
        l::CANCEL => l::CancelAck {
            id: id(),
            disposition: l::Disposition::Requested,
        }
        .packet(p.context)
        .unwrap(),
        _ => p,
    }
}

fn selected() -> files::Client {
    transport::reset();
    transport::respond(reply);
    let mut client = files::Client::new(1, 7, 11);
    client.select_lifecycle(Method::OperationsGet).unwrap();
    client.select_lifecycle(Method::OperationsCancel).unwrap();
    client
}

#[test]
fn two_methods_survive_other_exchanges_without_implicit_negotiation() {
    let mut client = selected();
    // Interleave an ordinary request on the same mutable client and endpoint.
    client
        .request(Packet::new(rustic_abi::files::ABORT))
        .unwrap();
    assert_eq!(
        client.inspect_selected(id()).unwrap().state,
        l::State::Prepared
    );
    assert_eq!(
        client.cancel_selected(id()).unwrap().disposition,
        l::Disposition::Requested
    );
    let sent = transport::requests();
    assert_eq!(
        sent.iter().map(|r| r.2.op).collect::<Vec<_>>(),
        [
            n::DESCRIBE,
            n::DESCRIBE,
            rustic_abi::files::ABORT,
            a::OBSERVE,
            l::CANCEL
        ]
    );
    assert!(sent.iter().all(|r| r.0 == 1 && r.2.context == 11));
}

#[test]
fn selection_does_not_enable_another_method_or_client() {
    let mut client = selected();
    let mut other = files::Client::new(2, 7, 11);
    assert_eq!(other.inspect_selected(id()), Err(E::Unavailable));
    client.rebind(1, 7, 11);
    client.select_lifecycle(Method::OperationsGet).unwrap();
    let before = transport::counts();
    assert_eq!(client.cancel_selected(id()), Err(E::Unavailable));
    assert_eq!(transport::counts(), before);
    assert!(client.inspect_selected(id()).is_ok());
}

#[test]
fn any_rebind_or_observed_context_change_invalidates_both_methods() {
    for mode in 0..3 {
        let mut client = selected();
        match mode {
            0 => client.rebind(1, 7, 11), // even identical numeric handles
            1 => client.rebind(2, 8, 12),
            _ => client.context = 12,
        }
        let before = transport::counts();
        assert_eq!(client.inspect_selected(id()), Err(E::Unavailable));
        client.context = 11; // restoring the number cannot restore cleared entries
        assert_eq!(client.cancel_selected(id()), Err(E::Unavailable));
        assert_eq!(transport::counts(), before);
    }
}

#[test]
fn context_change_during_new_selection_cannot_restore_old_method() {
    let mut client = selected();
    client.context = 12;
    client.select_lifecycle(Method::OperationsGet).unwrap();
    assert!(client.inspect_selected(id()).is_ok());
    assert_eq!(client.cancel_selected(id()), Err(E::Unavailable));
}

#[test]
fn failed_refresh_clears_prior_support_and_never_downgrades() {
    for fault in 0..6 {
        let mut client = selected();
        transport::respond(move |p| {
            let mut r = reply(p);
            match fault {
                0 => r.data[4] ^= 1, // wrong digest
                1 => r.version = 1,
                2 => r.context += 1,
                3 => {
                    r = Packet::new(n::DESCRIBE);
                    r.context = p.context;
                    r.status = E::Busy as u8;
                }
                4 => r.id = Method::OperationsCancel as u32,
                _ => (),
            }
            r
        });
        transport::wrong_peer(fault == 5);
        assert!(client.select_lifecycle(Method::OperationsGet).is_err());
        assert_eq!(transport::requests().len(), 3);
        assert_eq!(client.inspect_selected(id()), Err(E::Unavailable));
        assert_eq!(client.cancel_selected(id()), Err(E::Unavailable));
        assert_eq!(transport::requests().len(), 3);
    }
}

#[test]
fn unavailable_support_refuses_effect_without_sending() {
    let mut client = selected();
    transport::respond(|p| {
        let mut r = reply(p);
        r.data[0] = Availability::Unavailable as u8;
        r
    });
    assert_eq!(
        client
            .select_lifecycle(Method::OperationsCancel)
            .unwrap()
            .availability,
        Availability::Unavailable
    );
    assert_eq!(client.cancel_selected(id()), Err(E::Unavailable));
    assert_eq!(transport::requests().len(), 3);
}

#[test]
fn selected_methods_still_obey_fresh_service_denial() {
    for error in [E::Denied, E::Revoked] {
        let mut client = selected();
        transport::respond(move |p| {
            let mut r = Packet::new(p.op);
            r.context = p.context;
            r.status = error as u8;
            r
        });
        assert_eq!(client.inspect_selected(id()), Err(error));
        assert_eq!(client.cancel_selected(id()), Err(error));
        assert_eq!(transport::requests().len(), 4);
    }
}

#[test]
fn bad_cancel_reply_remains_uncertain_without_retry_or_followup_read() {
    for wrong_peer in [false, true] {
        let mut client = selected();
        transport::respond(|p| {
            let mut r = reply(p);
            r.version += 1;
            r
        });
        transport::wrong_peer(wrong_peer);
        assert_eq!(client.cancel_selected(id()), Err(E::Uncertain));
        assert_eq!(transport::requests().len(), 3);
        if wrong_peer {
            assert_eq!(client.cancel_selected(id()), Err(E::Uncertain));
            assert_eq!(
                transport::requests().len(),
                3,
                "poisoned RPC must not resend"
            );
        }
    }
}
