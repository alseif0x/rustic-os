// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
use rustic_abi::files::*;
use rustic_file_service::{Grant, Server};
use rustic_fs::{Disk, Volume};
#[derive(Clone)]
pub struct Memory(Vec<[u8; 512]>);
impl Disk for Memory {
    fn read(&mut self, s: u64, b: &mut [u8; 512]) -> Result<(), rustic_fs::Error> {
        *b = self.0[s as usize];
        Ok(())
    }
    fn write(&mut self, s: u64, b: &[u8; 512]) -> Result<(), rustic_fs::Error> {
        self.0[s as usize] = *b;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), rustic_fs::Error> {
        Ok(())
    }
}
pub fn setup() -> (Server, Memory, u32, u32) {
    let mut d = Memory(vec![[0; 512]; rustic_fs::SECTORS as usize]);
    let mut v = Volume::initialize(&mut d).unwrap();
    let a = v.create(&mut d, 4, b"a", rustic_fs::Kind::File).unwrap();
    let b = v.create(&mut d, 4, b"b", rustic_fs::Kind::File).unwrap();
    (Server::new(v), d, a.id, b.id)
}
pub fn grant(s: &mut Server, slot: usize, scope: u32, rights: u8, expires: u64) -> u32 {
    s.grant(
        slot,
        Grant {
            peer: slot as u64 + 10,
            endpoint: slot as u64 + 1,
            scope,
            rights,
            generation: 0,
            expires,
            subject: 0,
        },
    )
    .unwrap()
}
pub fn request(op: u8, id: u32, context: u32) -> Packet {
    Packet {
        id,
        context,
        ..Packet::new(op)
    }
}
pub fn run(s: &mut Server, d: &mut Memory, slot: usize, r: Packet, now: u64) -> Packet {
    s.handle(d, slot, slot as u64 + 10, r, now)
}
