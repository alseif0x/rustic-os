// SPDX-License-Identifier: Apache-2.0
//! Safe host IPC boundary for the actual native file client and RPC state machine.
use rustic_abi::files::Packet;
use rustic_sdk::ipc::Message;
use std::cell::RefCell;

#[derive(Default)]
struct Transport {
    sent: Vec<(u64, u64, Packet)>,
    reply: Option<Message>,
    clock_calls: usize,
    closed: Vec<u64>,
    wrong_peer: bool,
}
thread_local! {
    static TRANSPORT: RefCell<Transport> = RefCell::new(Transport::default());
}
pub fn reset() {
    TRANSPORT.with(|state| *state.borrow_mut() = Transport::default());
}
pub fn counts() -> (usize, usize, usize) {
    TRANSPORT.with(|state| {
        let state = state.borrow();
        (state.sent.len(), state.clock_calls, state.closed.len())
    })
}
pub fn requests() -> Vec<(u64, u64, Packet)> {
    TRANSPORT.with(|state| state.borrow().sent.clone())
}
pub fn wrong_peer(value: bool) {
    TRANSPORT.with(|state| state.borrow_mut().wrong_peer = value);
}

pub mod ipc {
    use super::TRANSPORT;
    use crate::Error;
    use rustic_abi::files::Packet;
    pub use rustic_sdk::ipc::Message;

    pub struct Endpoint(u64);
    impl Endpoint {
        pub fn from_bootstrap(token: u64) -> Self {
            Self(token)
        }
        pub fn token(&self) -> u64 {
            self.0
        }
        pub fn send(&self, message: &Message) -> Result<(), Error> {
            assert_ne!(self.0, 0, "unbound client reached IPC send");
            let packet = Packet::decode(message.payload()).unwrap();
            TRANSPORT.with(|state| {
                let mut state = state.borrow_mut();
                state.sent.push((self.0, message.correlation(), packet));
                // Canonical immediate response. Sender mismatch exercises the
                // production RPC poison/rebind state without native syscalls.
                let reply = Message::new(message.correlation(), &packet.encode()).unwrap();
                let mut bytes = reply.wire().to_vec();
                let peer = if state.wrong_peer { 8u64 } else { 7u64 };
                bytes[16..24].copy_from_slice(&peer.to_le_bytes());
                state.reply = Some(Message::from_received(&bytes).unwrap());
            });
            Ok(())
        }
        pub fn receive(&self) -> Result<Message, Error> {
            TRANSPORT.with(|state| {
                state
                    .borrow_mut()
                    .reply
                    .take()
                    .ok_or(Error::Ipc(rustic_abi::ipc::Error::WouldBlock))
            })
        }
        pub fn close(self) -> Result<(), Error> {
            TRANSPORT.with(|state| state.borrow_mut().closed.push(self.0));
            Ok(())
        }
    }
}

pub mod runtime {
    use super::TRANSPORT;
    pub fn clock() -> u64 {
        TRANSPORT.with(|state| state.borrow_mut().clock_calls += 1);
        0
    }
    pub fn wait_set(_: &[u64], _: u64) -> Result<(), rustic_abi::runtime::Error> {
        panic!("host fixture supplies immediate replies; waiting is a test failure")
    }
}
