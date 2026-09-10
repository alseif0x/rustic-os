// SPDX-License-Identifier: Apache-2.0
//! Native copied-sector adapter; completion identity and effects stay explicit.
use rustic_fs::Error;
use rustic_sdk::block::{Device, Operation, Status};
pub struct Disk(Device);
impl Disk {
    pub fn new(token: u64) -> Self {
        Self(Device::from_bootstrap(token))
    }
    fn complete(
        &self,
        id: Result<u64, rustic_sdk::block::Error>,
        operation: Operation,
    ) -> Result<[u8; 512], Error> {
        let id = id.map_err(|_| Error::Io)?;
        self.0.wait(id).map_err(|_| Error::Io)?;
        let c = self.0.result().map_err(|_| Error::Io)?;
        if c.id != id || c.operation != operation || c.status != Status::Success {
            return Err(Error::Io);
        }
        Ok(c.data)
    }
}
impl rustic_fs::Disk for Disk {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        *bytes = self.complete(self.0.read(sector), Operation::Read)?;
        Ok(())
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.complete(self.0.write(sector, bytes), Operation::Write)?;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        self.complete(self.0.flush(), Operation::Flush)?;
        Ok(())
    }
}
