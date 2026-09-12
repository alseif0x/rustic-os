// SPDX-License-Identifier: Apache-2.0
//! A volatile observation of execution; never a durable result or cancellation receipt.
use super::AdmissionId;
use crate::files::{Error, Packet, operation::Instance};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityPhase {
    Queued,
    Running,
    Stopping,
    Settling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activity {
    pub id: AdmissionId,
    pub service_instance: Instance,
    pub phase: ActivityPhase,
    /// Accepted in memory only. Query durable admission status after settlement.
    pub cancel_requested: bool,
    pub io_pending: bool,
}
impl Activity {
    fn validate(self, op: u8) -> Result<(), Error> {
        if !(super::live(op) || op == super::SCHEDULE)
            || self.service_instance.lineage() != self.id.lineage()
            || self.service_instance.sequence() > self.id.number()
            || (op == super::REQUEST_CANCEL || self.phase == ActivityPhase::Stopping)
                && !self.cancel_requested
            || self.phase == ActivityPhase::Running && self.cancel_requested
            || self.phase == ActivityPhase::Queued && self.io_pending
        {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    pub fn packet(self, op: u8, context: u32) -> Result<Packet, Error> {
        self.validate(op)?;
        let mut p = Packet::new(op);
        p.context = context;
        p.version = self.id.number();
        p.arg = match self.phase {
            ActivityPhase::Queued => 4,
            ActivityPhase::Running => 1,
            ActivityPhase::Stopping => 2,
            ActivityPhase::Settling => 3,
        } | ((self.cancel_requested as u32) << 8)
            | ((self.io_pending as u32) << 9);
        p.count = 24;
        p.data[..16].copy_from_slice(&self.id.lineage());
        p.data[16..24].copy_from_slice(&self.service_instance.sequence().to_le_bytes());
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if p.status != 0
            || p.id != 0
            || p.count != 24
            || p.data[24..] != [0; 16]
            || p.arg & !0x307 != 0
        {
            return Err(Error::Protocol);
        }
        let lineage = p.data[..16].try_into().unwrap();
        let result = Self {
            id: AdmissionId::new(lineage, p.version).map_err(|_| Error::Protocol)?,
            service_instance: Instance::new(
                lineage,
                u64::from_le_bytes(p.data[16..24].try_into().unwrap()),
            )
            .map_err(|_| Error::Protocol)?,
            phase: match p.arg & 7 {
                4 => ActivityPhase::Queued,
                1 => ActivityPhase::Running,
                2 => ActivityPhase::Stopping,
                3 => ActivityPhase::Settling,
                _ => return Err(Error::Protocol),
            },
            cancel_requested: p.arg & 0x100 != 0,
            io_pending: p.arg & 0x200 != 0,
        };
        result.validate(p.op)?;
        Ok(result)
    }
}
