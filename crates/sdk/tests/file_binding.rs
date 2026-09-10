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
use rustic_abi::files::{
    COMMIT, CREATE, Error as FileError, Packet, STAT, read::Request, reference::References,
};
pub use transport::{ipc, runtime};

fn read_request() -> Request {
    let refs = References::new([1; 16], 4, 6).unwrap();
    Request {
        workspace: refs.workspace,
        resource: refs.resource,
        expected_version: None,
        offset: 0,
        length: 3,
    }
}

#[test]
fn unbound_requests_and_observations_are_unavailable_without_transport_calls() {
    transport::reset();
    let mut client = files::Client::new(0, 0, 17);
    for operation in [STAT, CREATE, COMMIT] {
        assert_eq!(
            client.request(Packet::new(operation)),
            Err(FileError::Unavailable)
        );
        assert_eq!(
            client.submit(Packet::new(operation)),
            Err(FileError::Unavailable)
        );
        assert_eq!(client.poll(), Ok(None));
    }
    assert_eq!(client.references(4, 6), Err(FileError::Unavailable));
    let mut bytes = [0xa5; 8];
    assert_eq!(
        client.read_range(read_request(), &mut bytes),
        Err(FileError::Unavailable)
    );
    assert_eq!(bytes, [0; 8]);
    assert_eq!(transport::counts(), (0, 0, 0));
}

#[test]
fn fresh_binding_admits_requests_after_unavailable_and_can_be_removed_again() {
    transport::reset();
    let mut client = files::Client::new(0, 0, 17);
    assert_eq!(
        client.request(Packet::new(STAT)),
        Err(FileError::Unavailable)
    );
    client.rebind(9, 7, 33);
    assert_eq!(client.request(Packet::new(STAT)).unwrap().context, 33);
    client.submit(Packet::new(STAT)).unwrap();
    assert_eq!(client.poll().unwrap().unwrap().context, 33);
    let sent = transport::requests();
    assert_eq!((sent[0].0, sent[0].1, sent[1].0, sent[1].1), (9, 1, 9, 2));
    client.rebind(0, 0, 0);
    assert_eq!(
        client.request(Packet::new(COMMIT)),
        Err(FileError::Unavailable)
    );
    assert_eq!(transport::counts(), (2, 1, 2));
}

#[test]
fn poisoned_nonzero_binding_keeps_existing_errors_until_explicit_rebind() {
    transport::reset();
    transport::wrong_peer(true);
    let mut client = files::Client::new(9, 7, 33);
    assert_eq!(client.request(Packet::new(STAT)), Err(FileError::Protocol));
    assert_eq!(client.request(Packet::new(STAT)), Err(FileError::Protocol));
    assert_eq!(
        client.request(Packet::new(COMMIT)),
        Err(FileError::Uncertain)
    );
    assert_eq!(client.submit(Packet::new(STAT)), Err(FileError::Closed));
    assert_eq!(client.references(4, 6), Err(FileError::Protocol));
    assert_eq!(transport::requests().len(), 1);
    transport::wrong_peer(false);
    client.rebind(10, 7, 34);
    assert_eq!(client.request(Packet::new(STAT)).unwrap().context, 34);
    let sent = transport::requests();
    assert_eq!((sent[1].0, sent[1].1), (10, 1));
}
