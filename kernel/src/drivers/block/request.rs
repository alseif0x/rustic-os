// SPDX-License-Identifier: Apache-2.0
use super::device::Device;
use rustic_kernel::block::Error;
impl Device {
    pub(crate) fn read(&mut self, sector: u64, data: &mut [u8]) -> Result<(), Error> {
        self.geometry.validate(sector, data.len(), false)?;
        self.submit(0, sector, data, true)
    }
    pub(crate) fn write(&mut self, sector: u64, data: &[u8]) -> Result<(), Error> {
        self.geometry.validate(sector, data.len(), true)?;
        let mut copy = [0; 512];
        copy.copy_from_slice(data);
        self.submit(1, sector, &mut copy, true)
    }
    pub(crate) fn flush(&mut self) -> Result<(), Error> {
        self.submit(4, 0, &mut [], true)
    }
}

impl Device {
    pub(super) fn submit(
        &mut self,
        kind: u32,
        sector: u64,
        data: &mut [u8],
        notify: bool,
    ) -> Result<(), Error> {
        self.start(kind, sector, data, notify)?;
        loop {
            if let Some(result) = self.poll(data) {
                return result;
            }
            core::hint::spin_loop();
        }
    }
}
