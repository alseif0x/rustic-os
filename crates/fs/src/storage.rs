// SPDX-License-Identifier: Apache-2.0
use crate::{Error, MAX_FILE};
/// Implementations must honor successful flush ordering. Errors are not rollback.
pub trait Disk {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error>;
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error>;
    fn flush(&mut self) -> Result<(), Error>;
}
pub(super) const fn header(bank: u8) -> u64 {
    8 + bank as u64 * 5
}
pub(super) const fn data(slot: usize, bank: u8) -> u64 {
    32 + slot as u64 * 4 + bank as u64 * 2
}
pub(super) fn read_data(
    disk: &mut impl Disk,
    slot: usize,
    bank: u8,
) -> Result<[u8; MAX_FILE], Error> {
    let mut bytes = [0; MAX_FILE];
    for (i, part) in bytes.as_chunks_mut::<512>().0.iter_mut().enumerate() {
        disk.read(data(slot, bank) + i as u64, part)?;
    }
    Ok(bytes)
}
