// SPDX-License-Identifier: Apache-2.0
// Compile the private administrative decoder on the host; native IPC delivery of
// the same words remains separate evidence.
#[path = "../src/admin.rs"]
mod admin;

use rustic_file_service::Server;
use rustic_fs::{Disk, Volume};
use rustic_sdk::abi::files::{Error, GRANT, GRANT_SECOND_SCOPE, Packet, READ};

struct Memory(Vec<[u8; 512]>);
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

/// Two sibling files under `workspaces`: a document and a separate record.
fn setup() -> (Server, Memory, u32, u32) {
    let mut disk = Memory(vec![[0; 512]; rustic_fs::SECTORS as usize]);
    let mut volume = Volume::initialize(&mut disk).unwrap();
    let document = volume
        .create(&mut disk, 4, b"doc", rustic_fs::Kind::File)
        .unwrap()
        .id;
    let record = volume
        .create(&mut disk, 4, b"journal", rustic_fs::Kind::File)
        .unwrap()
        .id;
    (Server::new(volume), disk, document, record)
}

fn request(server: &mut Server, disk: &mut Memory, w: [u64; 8]) -> [u64; 8] {
    admin::dispatch(server, disk, w, 1)
}

#[test]
fn the_second_scope_opcode_extends_only_the_grant_it_names() {
    let (mut s, mut d, document, record) = setup();
    // [GRANT, slot, peer, endpoint, scope, rights, expires, subject]
    let granted = request(
        &mut s,
        &mut d,
        [GRANT as u64, 0, 10, 1, document as u64, 7, 0, 1],
    );
    assert_eq!(granted[0], 0);
    let generation = granted[1];
    assert!(generation != 0 && granted[2..].iter().all(|v| *v == 0));
    let read = |s: &mut Server, d: &mut Memory, id: u32| {
        s.handle(
            d,
            0,
            10,
            Packet {
                id,
                context: generation as u32,
                ..Packet::new(READ)
            },
            1,
        )
        .status
    };
    assert_eq!(read(&mut s, &mut d, record), Error::Denied as u8);
    // [GRANT_SECOND_SCOPE, slot, generation, object, 0, 0, 0, 0]
    let words = [
        GRANT_SECOND_SCOPE as u64,
        0,
        generation,
        record as u64,
        0,
        0,
        0,
        0,
    ];
    assert_eq!(
        request(&mut s, &mut d, words),
        [0, generation, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(read(&mut s, &mut d, document), 0);
    assert_eq!(read(&mut s, &mut d, record), 0);
}

#[test]
fn a_malformed_or_misdirected_extension_changes_nothing() {
    let (mut s, mut d, document, record) = setup();
    let generation = request(
        &mut s,
        &mut d,
        [GRANT as u64, 0, 10, 1, document as u64, 7, 0, 1],
    )[1];
    let base = [
        GRANT_SECOND_SCOPE as u64,
        0,
        generation,
        record as u64,
        0,
        0,
        0,
        0,
    ];
    // Unused words must stay zero; the opcode carries nothing else.
    let mut noisy = base;
    noisy[6] = 1;
    assert_eq!(
        request(&mut s, &mut d, noisy)[0],
        Error::Protocol as u8 as u64
    );
    // Another slot and another generation are refused.
    let mut elsewhere = base;
    elsewhere[1] = 1;
    assert_eq!(
        request(&mut s, &mut d, elsewhere)[0],
        Error::Denied as u8 as u64
    );
    let mut stale = base;
    stale[2] = generation + 1;
    assert_eq!(
        request(&mut s, &mut d, stale)[0],
        Error::Denied as u8 as u64
    );
    // The named object is still validated by the service.
    let mut inside = base;
    inside[3] = 4;
    assert_eq!(
        request(&mut s, &mut d, inside)[0],
        Error::Invalid as u8 as u64
    );
    assert_eq!(
        request(&mut s, &mut d, base),
        [0, generation, 0, 0, 0, 0, 0, 0]
    );
}
