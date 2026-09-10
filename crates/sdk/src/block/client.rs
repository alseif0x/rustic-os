// SPDX-License-Identifier: Apache-2.0
use crate::arch;
use rustic_abi::block::*;
/// Explicitly provisioned owner-bound token, never a path or authority request.
pub struct Device(u64);
impl Device {
    pub fn from_bootstrap(token: u64) -> Self {
        Self(token)
    }
    pub fn version() -> Result<u16, Error> {
        // SAFETY: All-zero INFO arguments contain no pointer or ownership claim.
        let version = Error::decode(unsafe { arch::call(INFO, 0, 0, 0) })?;
        u16::try_from(version).map_err(|_| Error::Protocol)
    }
    pub fn geometry(&self) -> Result<Geometry, Error> {
        let mut bytes = [0; GEOMETRY_BYTES];
        // SAFETY: Exclusive live output allocation, retained for immediate copy-out only.
        let count = Error::decode(unsafe {
            arch::call(INFO, self.0, bytes.as_mut_ptr() as u64, bytes.len() as u64)
        })?;
        if count != GEOMETRY_BYTES as u64 {
            return Err(Error::Protocol);
        }
        Geometry::decode(&bytes)
    }
    fn submit(&self, request: Request) -> Result<u64, Error> {
        let bytes = request.encode();
        // SAFETY: Descriptor is initialized and live until synchronous admission returns.
        // A write's input address is supplied only by write() with its live slice.
        let id = Error::decode(unsafe {
            arch::call(SUBMIT, self.0, bytes.as_ptr() as u64, bytes.len() as u64)
        })?;
        if id == 0 {
            return Err(Error::Protocol);
        }
        Ok(id)
    }
    pub fn read(&self, sector: u64) -> Result<u64, Error> {
        self.submit(Request {
            operation: Operation::Read,
            sector,
            address: 0,
            length: SECTOR as u32,
        })
    }
    pub fn write(&self, sector: u64, bytes: &[u8; SECTOR]) -> Result<u64, Error> {
        // The kernel snapshots all bytes before submit returns; no borrow survives it.
        self.submit(Request {
            operation: Operation::Write,
            sector,
            address: bytes.as_ptr() as u64,
            length: SECTOR as u32,
        })
    }
    pub fn flush(&self) -> Result<u64, Error> {
        self.submit(Request {
            operation: Operation::Flush,
            sector: 0,
            address: 0,
            length: 0,
        })
    }
    pub fn result(&self) -> Result<Completion, Error> {
        let mut bytes = [0; RESULT_BYTES];
        // SAFETY: Full exclusive output allocation; completion is consumed only after copy-out.
        let count = Error::decode(unsafe {
            arch::call(
                RESULT,
                self.0,
                bytes.as_mut_ptr() as u64,
                bytes.len() as u64,
            )
        })?;
        if count != RESULT_BYTES as u64 {
            return Err(Error::Protocol);
        }
        Completion::decode(&bytes)
    }
    pub fn wait(&self, id: u64) -> Result<(), Error> {
        // SAFETY: Integer identity arguments only; blocking retains no user pointer.
        if Error::decode(unsafe { arch::call(WAIT, self.0, id, 0) })? != 0 {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    /// True means cancelled before submission; false means too late, query the result.
    pub fn cancel(&self, id: u64) -> Result<bool, Error> {
        // SAFETY: Integer identity arguments only.
        match Error::decode(unsafe { arch::call(CANCEL, self.0, id, 0) })? {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(Error::Protocol),
        }
    }
    pub fn close(self) -> Result<(), Error> {
        // SAFETY: Integer token only. Kernel retains abandoned device-owned storage.
        if Error::decode(unsafe { arch::call(CLOSE, self.0, 0, 0) })? != 0 {
            return Err(Error::Protocol);
        }
        Ok(())
    }
}
