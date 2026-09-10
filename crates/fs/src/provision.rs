// SPDX-License-Identifier: Apache-2.0
//! R0 trusted provisioning envelope. The host supplies lineage; it supplies no file state.
use crate::{Disk, Error, checksum::crc};
pub(crate) fn lineage(disk: &mut impl Disk) -> Result<Option<[u8; 16]>, Error> {
    let mut b = [0; 512];
    disk.read(1, &mut b)?;
    if b == [0; 512] {
        return Ok(None);
    }
    let expected = u32::from_le_bytes(b[24..28].try_into().unwrap());
    b[24..28].fill(0);
    if &b[..8] != b"RUSTVOL1"
        || b[8..24] == [0; 16]
        || b[28..].iter().any(|v| *v != 0)
        || expected != crc(&b)
    {
        return Err(Error::Corrupt);
    }
    Ok(Some(b[8..24].try_into().unwrap()))
}
