// SPDX-License-Identifier: Apache-2.0
//! Actual file/RPC code over a safe host transport; no native syscall execution.
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
use rustic_abi::files::Error as FileError;
use rustic_abi::files::{operation::*, reference::*, *};
use sha2::{Digest, Sha256};
pub use transport::{ipc, runtime};
fn expected() -> (Replacement, Operation) {
    let workspace = Workspace::new([7; 16], 4).unwrap();
    let resource = Resource::new(workspace, 6).unwrap();
    let retry = Retry {
        epoch: Epoch::new(1).unwrap(),
        key: Key::new(42).unwrap(),
    };
    let request = Replacement {
        workspace,
        resource,
        expected_version: Version::new(2).unwrap(),
        retry,
    };
    let result = Operation {
        id: OperationId::new([7; 16], 9).unwrap(),
        service_instance: Instance::new([7; 16], 9).unwrap(),
        workspace,
        resource,
        previous_version: request.expected_version,
        version: Version::new(9).unwrap(),
        size: 5,
        retry,
        sha256: Sha256::digest(b"after").into(),
    };
    (request, result)
}
fn responder(result: Operation, corrupt: u8) {
    transport::reset();
    transport::respond(move |p| {
        let mut r = if matches!(
            p.op,
            REPLACE_COMMIT | OPERATION_RETRY | OPERATION_ID | OPERATION_PART
        ) {
            result
                .part(
                    p.op,
                    p.context,
                    if p.op == OPERATION_PART {
                        p.arg as usize
                    } else {
                        0
                    },
                )
                .unwrap()
        } else {
            let mut r = Packet::new(p.op);
            r.context = p.context;
            r
        };
        if p.op == OPERATION_PART && p.arg == 40 {
            match corrupt {
                1 => r.version += 1,
                2 => r.id = 0,
                3 => r.arg += 1,
                4 => {
                    r = Packet::new(p.op);
                    r.context = p.context;
                    r.status = FileError::Denied as u8;
                }
                5 => r.data[26] = 1, // Reserved receipt byte 66.
                _ => (),
            }
        }
        r
    });
}
#[test]
fn native_client_collects_one_consistent_receipt_and_does_not_replay_the_mutation() {
    let (request, result) = expected();
    responder(result, 0);
    let mut client = files::Client::new(1, 7, 11);
    assert_eq!(client.replace_file(request, b"after"), Ok(result));
    assert_eq!(
        client.operation_get(Lookup::Retry {
            workspace: request.workspace,
            retry: request.retry
        }),
        Ok(result)
    );
    assert_eq!(client.operation_get(Lookup::Id(result.id)), Ok(result));
    assert_eq!(
        transport::requests()
            .iter()
            .filter(|(_, _, p)| p.op == REPLACE_COMMIT)
            .count(),
        1
    );
    assert_eq!(
        transport::requests()
            .iter()
            .filter(|(_, _, p)| p.op == OPERATION_PART)
            .count(),
        6
    );
}
#[test]
fn corrupt_or_revoked_fragments_leave_the_mutation_uncertain_and_queries_fail_closed() {
    let (request, result) = expected();
    for corruption in 1..=5 {
        responder(result, corruption);
        let mut client = files::Client::new(1, 7, 11);
        assert_eq!(
            client.replace_file(request, b"after"),
            Err(FileError::Uncertain)
        );
        assert_eq!(
            transport::requests()
                .iter()
                .filter(|(_, _, p)| p.op == REPLACE_COMMIT)
                .count(),
            1
        );
        responder(result, corruption);
        let mut client = files::Client::new(1, 7, 11);
        assert_eq!(
            client.operation_get(Lookup::Id(result.id)),
            Err(if corruption == 4 {
                FileError::Denied
            } else {
                FileError::Protocol
            })
        );
    }
    let mut wrong_hash = result;
    wrong_hash.sha256[0] ^= 1;
    responder(wrong_hash, 0);
    assert_eq!(
        files::Client::new(1, 7, 11).replace_file(request, b"after"),
        Err(FileError::Uncertain)
    );
    let mut wrong_namespace = result;
    wrong_namespace.retry.key = Key::new(43).unwrap();
    responder(wrong_namespace, 0);
    assert_eq!(
        files::Client::new(1, 7, 11).operation_get(Lookup::Retry {
            workspace: request.workspace,
            retry: request.retry
        }),
        Err(FileError::Protocol)
    );
}
#[test]
fn busy_open_does_not_abort_a_previous_transfer_and_unbound_operations_do_not_send() {
    let (request, result) = expected();
    transport::reset();
    transport::respond(|p| {
        let mut r = Packet::new(p.op);
        r.context = p.context;
        r.status = FileError::Busy as u8;
        r
    });
    assert_eq!(
        files::Client::new(1, 7, 11).stage_replace(request, b"after"),
        Err(FileError::Busy)
    );
    assert_eq!(transport::requests().len(), 1);
    transport::reset();
    let mut client = files::Client::new(0, 0, 0);
    assert_eq!(
        client.replace_file(request, b"after"),
        Err(FileError::Unavailable)
    );
    assert_eq!(
        client.operation_get(Lookup::Id(result.id)),
        Err(FileError::Unavailable)
    );
    assert_eq!(transport::counts(), (0, 0, 0));
}
