// SPDX-License-Identifier: Apache-2.0
//! Native copied-sector adapter; completion identity and effects stay explicit.
use rustic_fs::Error;
use rustic_sdk::block::{Device, Operation, Status};
mod poll;
pub struct Disk {
    device: Device,
    pending: Option<poll::Pending>,
    fenced: bool,
}
impl Disk {
    pub fn new(token: u64) -> Self {
        Self {
            device: Device::from_bootstrap(token),
            pending: None,
            fenced: false,
        }
    }
    fn ready(&self) -> Result<(), Error> {
        if self.fenced || self.pending.is_some() {
            Err(Error::Uncertain)
        } else {
            Ok(())
        }
    }
    fn complete(
        &self,
        id: Result<u64, rustic_sdk::block::Error>,
        operation: Operation,
    ) -> Result<[u8; 512], Error> {
        let id = id.map_err(|_| Error::Io)?;
        self.device.wait(id).map_err(|_| Error::Io)?;
        let c = self.device.result().map_err(|_| Error::Io)?;
        if c.id != id || c.operation != operation || c.status != Status::Success {
            return Err(Error::Io);
        }
        Ok(c.data)
    }
}
impl rustic_fs::Disk for Disk {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), Error> {
        self.ready()?;
        *bytes = self.complete(self.device.read(sector), Operation::Read)?;
        Ok(())
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), Error> {
        self.ready()?;
        self.complete(self.device.write(sector, bytes), Operation::Write)?;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        self.ready()?;
        self.complete(self.device.flush(), Operation::Flush)?;
        Ok(())
    }
}
