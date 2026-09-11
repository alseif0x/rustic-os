// SPDX-License-Identifier: Apache-2.0
use super::AdmissionId;
use crate::files::{
    Error, Packet,
    operation::{Instance, OperationId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Admitted,
    Cancelled,
    Committed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub id: AdmissionId,
    pub state: State,
    /// Originating durable service incarnation, not a live authority token.
    pub service_instance: Instance,
    /// Zero while admitted; durable terminal transaction otherwise.
    pub terminal: u64,
}
impl Status {
    fn validate(self) -> Result<(), Error> {
        if self.service_instance.lineage() != self.id.lineage()
            || self.service_instance.sequence() > self.id.number()
            || match self.state {
                State::Admitted => self.terminal != 0,
                State::Cancelled | State::Committed => self.terminal <= self.id.number(),
            }
        {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    /// The full completion receipt requires a separate INSPECT-authorized lookup.
    pub fn completion(self) -> Option<OperationId> {
        if self.state == State::Committed {
            OperationId::new(self.id.lineage(), self.terminal).ok()
        } else {
            None
        }
    }
    pub fn packet(self, op: u8, context: u32) -> Result<Packet, Error> {
        self.validate()?;
        if !matches!(
            op,
            super::ACCEPT | super::GET | super::RETRY | super::EXECUTE | super::CANCEL
        ) {
            return Err(Error::Protocol);
        }
        let mut p = Packet::new(op);
        p.context = context;
        p.arg = match self.state {
            State::Admitted => 1,
            State::Cancelled => 2,
            State::Committed => 3,
        };
        p.version = self.id.number();
        p.count = 32;
        p.data[..16].copy_from_slice(&self.id.lineage());
        p.data[16..24].copy_from_slice(&self.service_instance.sequence().to_le_bytes());
        p.data[24..32].copy_from_slice(&self.terminal.to_le_bytes());
        Ok(p)
    }
    pub fn decode(p: &Packet) -> Result<Self, Error> {
        if !matches!(
            p.op,
            super::ACCEPT | super::GET | super::RETRY | super::EXECUTE | super::CANCEL
        ) || p.status != 0
            || p.id != 0
            || p.count != 32
            || p.data[32..] != [0; 8]
        {
            return Err(Error::Protocol);
        }
        let lineage = p.data[..16].try_into().unwrap();
        let result = Self {
            id: AdmissionId::new(lineage, p.version).map_err(|_| Error::Protocol)?,
            state: match p.arg {
                1 => State::Admitted,
                2 => State::Cancelled,
                3 => State::Committed,
                _ => return Err(Error::Protocol),
            },
            service_instance: Instance::new(
                lineage,
                u64::from_le_bytes(p.data[16..24].try_into().unwrap()),
            )
            .map_err(|_| Error::Protocol)?,
            terminal: u64::from_le_bytes(p.data[24..32].try_into().unwrap()),
        };
        result.validate()?;
        Ok(result)
    }
}
