// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, READ_RIGHT, WRITE_RIGHT};
use rustic_fs::Volume;
pub const CLIENTS: usize = 4;
#[derive(Clone, Copy, Debug)]
pub struct Grant {
    pub peer: u64,
    pub endpoint: u64,
    pub scope: u32,
    pub rights: u8,
    pub generation: u32,
    pub expires: u64,
}
impl Grant {
    pub(super) fn check(&self, peer: u64, context: u32, now: u64) -> Result<(), Error> {
        if self.peer != peer {
            return Err(Error::Denied);
        }
        if self.generation != context || self.rights == 0 {
            return Err(Error::Revoked);
        }
        if self.expires != 0 && now >= self.expires {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub(super) fn access(&self, volume: &Volume, id: u32, write: bool) -> Result<(), Error> {
        let right = if write { WRITE_RIGHT } else { READ_RIGHT };
        if self.rights & right == 0
            || !(id == 0 && self.scope == 0 && !write || volume.within(id, self.scope))
        {
            Err(Error::Denied)
        } else {
            Ok(())
        }
    }
}
