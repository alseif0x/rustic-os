// SPDX-License-Identifier: Apache-2.0
//! Application admission contract. Capability bits are requests, never grants.
pub const SIZE: usize = 128;
pub const VERSION: u16 = 1;
pub const IPC: u64 = 1;
pub const DIAGNOSTIC: u64 = 2;
pub const BLOCK: u64 = 4;
pub const KNOWN: u64 = IPC | DIAGNOSTIC | BLOCK;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Length,
    Magic,
    Version,
    Abi,
    Ipc,
    Reserved,
    Identity,
    Executable,
    Capabilities,
}

#[derive(Debug)]
pub struct Manifest<'a> {
    pub identity: &'a str,
    pub executable: &'a str,
    pub version: [u16; 3],
    pub requests: u64,
}
fn name(bytes: &[u8]) -> Option<&str> {
    let end = bytes.iter().position(|b| *b == 0)?;
    let value = &bytes[..end];
    if value.is_empty()
        || !value[0].is_ascii_lowercase()
        || !value
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(b))
        || bytes[end..].iter().any(|b| *b != 0)
    {
        return None;
    }
    core::str::from_utf8(value).ok()
}
impl<'a> Manifest<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() != SIZE {
            return Err(Error::Length);
        }
        if &bytes[..8] != b"RUSTAPP\0" {
            return Err(Error::Magic);
        }
        let u16_at = |i| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if u16_at(8) != VERSION || usize::from(u16_at(10)) != SIZE {
            return Err(Error::Version);
        }
        if u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as u64 != crate::process::VERSION {
            return Err(Error::Abi);
        }
        if u16_at(16) != crate::ipc::VERSION {
            return Err(Error::Ipc);
        }
        if bytes[96..].iter().any(|b| *b != 0) {
            return Err(Error::Reserved);
        }
        let identity = name(&bytes[32..64]).ok_or(Error::Identity)?;
        let executable = name(&bytes[64..96])
            .filter(|s| s.ends_with(".elf"))
            .ok_or(Error::Executable)?;
        let requests = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        if requests & !KNOWN != 0 {
            return Err(Error::Capabilities);
        }
        Ok(Self {
            identity,
            executable,
            version: [u16_at(18), u16_at(20), u16_at(22)],
            requests,
        })
    }
    /// Admission only. The caller must separately supply actual endpoint authority.
    pub fn admitted(&self, available: u64) -> bool {
        self.requests & !available == 0
    }
}
