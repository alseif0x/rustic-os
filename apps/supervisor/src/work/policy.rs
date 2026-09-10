// SPDX-License-Identifier: Apache-2.0
//! Bounded policy loading/initialization over asynchronous native file requests.
use rustic_sdk::abi::files as f;
use rustic_sdk::files::{Client, Error, Metadata, Packet};
const CONTENT: &[u8] = b"rustic-owner-v1\nhelpers=explicit\n";
enum Phase {
    Lookup,
    Create,
    Begin,
    Chunk,
    Commit,
    Read,
}
pub(super) struct Policy {
    phase: Phase,
    sent: bool,
    id: u32,
    version: u64,
}
impl Policy {
    pub fn new() -> Self {
        Self {
            phase: Phase::Lookup,
            sent: false,
            id: 0,
            version: 0,
        }
    }
    pub fn poll(&mut self, files: &mut Client) -> Option<u32> {
        if !self.sent {
            let mut p = Packet::new(match self.phase {
                Phase::Lookup => f::LOOKUP,
                Phase::Create => f::CREATE,
                Phase::Begin => f::BEGIN,
                Phase::Chunk => f::CHUNK,
                Phase::Commit => f::COMMIT,
                Phase::Read => f::READ,
            });
            p.id = self.id;
            match self.phase {
                Phase::Lookup | Phase::Create => {
                    p.id = 3;
                    p.count = 12;
                    p.data[..12].copy_from_slice(b"owner-policy");
                }
                Phase::Begin => {
                    p.arg = CONTENT.len() as u32;
                    p.version = self.version;
                }
                Phase::Chunk => {
                    p.count = CONTENT.len() as u8;
                    p.data[..CONTENT.len()].copy_from_slice(CONTENT);
                }
                Phase::Read => {
                    p.version = self.version;
                }
                Phase::Commit => {}
            }
            match files.submit(p) {
                Ok(()) => self.sent = true,
                Err(Error::Busy) => {}
                Err(_) => return Some(0),
            }
            return None;
        }
        let r = match files.poll() {
            Ok(None) => return None,
            Ok(Some(r)) => r,
            Err(Error::NotFound) if matches!(self.phase, Phase::Lookup) => {
                self.phase = Phase::Create;
                self.sent = false;
                return None;
            }
            Err(_) => return Some(0),
        };
        self.sent = false;
        match self.phase {
            Phase::Lookup | Phase::Create | Phase::Commit => {
                let Ok(m) = Metadata::decode(r) else {
                    return Some(0);
                };
                if m.directory {
                    return Some(0);
                }
                self.id = m.id;
                self.version = m.version;
                if matches!(self.phase, Phase::Create) {
                    self.phase = Phase::Begin;
                } else if m.length == CONTENT.len() {
                    self.phase = Phase::Read;
                } else {
                    return Some(0);
                }
            }
            Phase::Begin => self.phase = Phase::Chunk,
            Phase::Chunk => self.phase = Phase::Commit,
            Phase::Read => {
                return Some(
                    if r.id == self.id
                        && r.version == self.version
                        && r.arg == CONTENT.len() as u32
                        && r.payload() == CONTENT
                    {
                        self.id
                    } else {
                        0
                    },
                );
            }
        }
        None
    }
}
