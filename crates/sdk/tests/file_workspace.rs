// SPDX-License-Identifier: Apache-2.0
//! Profile-2 streamed replacement client over a safe host transport.
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
use rustic_abi::files::{
    operation::{Instance, Key, Lookup as IdentityLookup, OperationId, Replacement, Retry},
    reference::*,
    workspace::{self, Lookup, Operation},
    *,
};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::rc::Rc;
pub use transport::{ipc, runtime};

fn pattern(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| (index * 7 + index / 509) as u8)
        .collect()
}

fn request() -> Replacement {
    let workspace = Workspace::new([7; 16], 5).unwrap();
    Replacement {
        workspace,
        resource: Resource::new(workspace, 6).unwrap(),
        expected_version: Version::new(2).unwrap(),
        retry: Retry {
            epoch: Epoch::new(1).unwrap(),
            key: Key::new(42).unwrap(),
        },
    }
}

fn receipt(request: Replacement, bytes: &[u8]) -> Operation {
    Operation {
        id: OperationId::new([7; 16], 9).unwrap(),
        service_instance: Instance::new([7; 16], 9).unwrap(),
        workspace: request.workspace,
        resource: request.resource,
        previous_version: request.expected_version,
        version: Version::new(9).unwrap(),
        size: bytes.len() as u32,
        retry: request.retry,
        sha256: Sha256::digest(bytes).into(),
    }
}

/// A service stand-in that accumulates chunks and answers the commit with a
/// receipt over whatever it received, optionally altered by `tamper`.
fn service(tamper: fn(&mut Operation)) -> Rc<RefCell<Vec<u8>>> {
    transport::reset();
    let received = Rc::new(RefCell::new(Vec::new()));
    let store = received.clone();
    let mut operation = None::<Operation>;
    transport::respond(move |p| {
        let mut reply = Packet::new(p.op);
        reply.context = p.context;
        match p.op {
            REPLACE_OPEN => {
                assert_eq!(p.count, 40, "profile-2 open carries the marker");
                store.borrow_mut().clear();
            }
            REPLACE_CHUNK => {
                assert_eq!(p.arg as usize, store.borrow().len());
                store.borrow_mut().extend_from_slice(p.payload());
            }
            REPLACE_COMMIT => {
                let mut result = receipt(request(), &store.borrow());
                tamper(&mut result);
                operation = Some(result);
                return result.part(REPLACE_COMMIT, p.context, 0).unwrap();
            }
            OPERATION_PART => {
                let IdentityLookup::Id(id) = Lookup::decode(&p).unwrap().query else {
                    panic!("receipt parts are looked up by id");
                };
                let result = operation.unwrap();
                assert_eq!(id, result.id);
                return result
                    .part(OPERATION_PART, p.context, p.arg as usize)
                    .unwrap();
            }
            REPLACE_ABORT => {}
            op => panic!("unexpected op {op}"),
        }
        reply
    });
    received
}

fn ops() -> Vec<u8> {
    transport::requests().iter().map(|(_, _, p)| p.op).collect()
}

#[test]
fn a_large_file_is_streamed_from_the_source_and_its_receipt_is_verified() {
    let bytes = pattern(8 * 1024 + 3);
    let received = service(|_| ());
    let mut client = files::Client::new(1, 7, 11);
    let mut offsets = Vec::new();
    let result = client
        .workspace_replace(request(), bytes.len() as u32, |offset, buffer| {
            offsets.push(offset);
            let start = offset as usize;
            buffer.copy_from_slice(&bytes[start..start + buffer.len()]);
            Ok(())
        })
        .unwrap();
    assert_eq!(result, receipt(request(), &bytes));
    assert_eq!(*received.borrow(), bytes);
    let chunks = bytes.len().div_ceil(DATA);
    assert_eq!(
        offsets,
        (0..chunks).map(|i| (i * DATA) as u32).collect::<Vec<_>>()
    );
    let ops = ops();
    assert_eq!(ops.first(), Some(&REPLACE_OPEN));
    assert_eq!(
        ops.iter().filter(|op| **op == REPLACE_CHUNK).count(),
        chunks
    );
    assert_eq!(
        &ops[ops.len() - 3..],
        &[REPLACE_COMMIT, OPERATION_PART, OPERATION_PART]
    );
}

#[test]
fn a_source_failure_aborts_before_commit() {
    service(|_| ());
    let mut client = files::Client::new(1, 7, 11);
    let result = client.workspace_replace(request(), 1000, |offset, buffer| {
        if offset >= 400 {
            return Err(FileError::Io);
        }
        buffer.fill(1);
        Ok(())
    });
    assert_eq!(result, Err(FileError::Io));
    let ops = ops();
    assert_eq!(ops.last(), Some(&REPLACE_ABORT));
    assert!(!ops.contains(&REPLACE_COMMIT));
}

#[test]
fn a_receipt_that_does_not_match_the_streamed_bytes_is_uncertain() {
    let bytes = pattern(513);
    for tamper in [
        (|result: &mut Operation| result.sha256[0] ^= 1) as fn(&mut Operation),
        |result| result.size -= 1,
        |result| result.previous_version = Version::new(1).unwrap(),
        |result| result.retry.key = Key::new(43).unwrap(),
    ] {
        service(tamper);
        let mut client = files::Client::new(1, 7, 11);
        assert_eq!(
            client.workspace_replace(request(), bytes.len() as u32, |offset, buffer| {
                let start = offset as usize;
                buffer.copy_from_slice(&bytes[start..start + buffer.len()]);
                Ok(())
            }),
            Err(FileError::Uncertain)
        );
        assert_eq!(ops().iter().filter(|op| **op == REPLACE_COMMIT).count(), 1);
    }
}

#[test]
fn oversized_files_are_refused_without_a_request() {
    transport::reset();
    let mut client = files::Client::new(1, 7, 11);
    assert_eq!(
        client.workspace_replace(request(), workspace::MAX_FILE_BYTES + 1, |_, _| Ok(())),
        Err(FileError::Size)
    );
    assert_eq!(transport::counts(), (0, 0, 0));
}
