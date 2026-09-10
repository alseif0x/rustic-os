// SPDX-License-Identifier: Apache-2.0
use super::Grant;
use crate::{block::Geometry, handles::Table};
use rustic_abi::block::{ALL, Completion, Effect, Error, MAX_ID, Operation, SECTOR, Status};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Queued,
    Active,
    Complete,
}
#[derive(Clone, Copy)]
struct Record {
    owner: u64,
    handle: u64,
    sector: u64,
    phase: Phase,
    abandoned: bool,
    result: Completion,
}
/// Owned snapshot returned to the sole device dispatcher, never to untrusted code.
pub struct Pending {
    pub id: u64,
    pub operation: Operation,
    pub sector: u64,
    pub data: [u8; SECTOR],
}
pub struct Broker {
    handles: Table<Grant, 4, 1>,
    records: [Option<Record>; 2],
    next: u64,
}
impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}
impl Broker {
    pub const fn new() -> Self {
        Self {
            handles: Table::new(1, ALL, 0),
            records: [None; 2],
            next: 1,
        }
    }
    pub fn grant(&mut self, owner: u64, grant: Grant, capacity: u64) -> Result<u64, Error> {
        grant.validate(capacity)?;
        Ok(self.handles.grant(owner, grant, grant.rights)?)
    }
    pub fn geometry(
        &self,
        owner: u64,
        handle: u64,
        physical: Geometry,
    ) -> Result<rustic_abi::block::Geometry, Error> {
        let grant = self.handles.resolve(owner, handle, 0)?;
        Ok(rustic_abi::block::Geometry {
            sectors: grant.sectors,
            rights: grant.rights,
            read_only: physical.read_only,
        })
    }
    pub fn check(
        &self,
        owner: u64,
        handle: u64,
        operation: Operation,
        relative: u64,
        physical: Geometry,
    ) -> Result<u64, Error> {
        let grant = self.handles.resolve(owner, handle, operation.right())?;
        let sector = if operation == Operation::Flush {
            0
        } else {
            grant.sector(relative)?
        };
        if operation != Operation::Read && physical.read_only {
            return Err(Error::ReadOnly);
        }
        Ok(sector)
    }
    pub fn admit(
        &mut self,
        owner: u64,
        handle: u64,
        operation: Operation,
        sector: u64,
        data: [u8; SECTOR],
        physical: Geometry,
    ) -> Result<u64, Error> {
        let sector = self.check(owner, handle, operation, sector, physical)?;
        if self.records.iter().flatten().any(|r| r.owner == owner) {
            return Err(Error::Busy);
        }
        let slot = self
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Busy)?;
        let next = self
            .next
            .checked_add(1)
            .filter(|n| *n <= MAX_ID)
            .ok_or(Error::Quota)?;
        let id = self.next;
        self.next = next;
        self.records[slot] = Some(Record {
            owner,
            handle,
            sector,
            phase: Phase::Queued,
            abandoned: false,
            result: Completion {
                id,
                operation,
                status: Status::Success,
                effect: Effect::None,
                data,
            },
        });
        Ok(id)
    }
    pub fn start(&mut self) -> Option<Pending> {
        if self
            .records
            .iter()
            .flatten()
            .any(|r| r.phase == Phase::Active)
        {
            return None;
        }
        let record = self
            .records
            .iter_mut()
            .flatten()
            .filter(|r| r.phase == Phase::Queued)
            .min_by_key(|r| r.result.id)?;
        record.phase = Phase::Active;
        Some(Pending {
            id: record.result.id,
            operation: record.result.operation,
            sector: record.sector,
            data: record.result.data,
        })
    }
    pub fn finish(&mut self, id: u64, status: Status, data: [u8; SECTOR]) {
        let slot = self
            .records
            .iter()
            .position(|r| r.is_some_and(|r| r.result.id == id && r.phase == Phase::Active))
            .expect("sole active request");
        let record = self.records[slot].as_mut().unwrap();
        if record.abandoned {
            self.records[slot] = None;
            return;
        }
        record.phase = Phase::Complete;
        record.result.status = status;
        record.result.effect =
            if record.result.operation == Operation::Read || status == Status::Unavailable {
                Effect::None
            } else if status == Status::Success {
                Effect::Completed
            } else {
                Effect::Unknown
            };
        record.result.data =
            if record.result.operation == Operation::Read && status == Status::Success {
                data
            } else {
                [0; SECTOR]
            };
    }
    fn slot(&self, owner: u64, handle: u64) -> Result<usize, Error> {
        self.handles.resolve(owner, handle, 0)?;
        self.records
            .iter()
            .position(|r| r.is_some_and(|r| r.owner == owner && r.handle == handle))
            .ok_or(Error::NoRequest)
    }
    pub fn peek(&self, owner: u64, handle: u64) -> Result<Completion, Error> {
        let record = self.records[self.slot(owner, handle)?].unwrap();
        if record.phase != Phase::Complete {
            return Err(Error::WouldBlock);
        }
        Ok(record.result)
    }
    pub fn wait(&self, owner: u64, handle: u64, id: u64) -> Result<(), Error> {
        let record = self.records[self.slot(owner, handle)?].unwrap();
        if record.result.id != id {
            return Err(Error::NoRequest);
        }
        self.peek(owner, handle).map(|_| ())
    }
    pub fn consume(&mut self, owner: u64, handle: u64) -> Result<(), Error> {
        self.peek(owner, handle)?;
        let slot = self.slot(owner, handle)?;
        self.records[slot] = None;
        Ok(())
    }
    pub fn cancel(&mut self, owner: u64, handle: u64, id: u64) -> Result<u64, Error> {
        let slot = self.slot(owner, handle)?;
        let record = self.records[slot].as_mut().unwrap();
        if record.result.id != id {
            return Err(Error::NoRequest);
        }
        if record.phase != Phase::Queued {
            return Ok(1);
        }
        record.phase = Phase::Complete;
        record.result.status = Status::Cancelled;
        record.result.data = [0; SECTOR];
        Ok(0)
    }
    pub fn close(&mut self, owner: u64, handle: u64) -> Result<(), Error> {
        self.handles.remove(owner, handle)?;
        for slot in &mut self.records {
            if let Some(record) = slot
                && record.handle == handle
            {
                if record.phase == Phase::Active {
                    record.abandoned = true;
                } else {
                    *slot = None;
                }
            }
        }
        Ok(())
    }
    pub fn close_owner(&mut self, owner: u64) {
        while let Some(handle) = self.handles.first(owner) {
            self.close(owner, handle).expect("owned handle");
        }
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.handles.count(), self.records.iter().flatten().count())
    }
    pub fn active(&self) -> Option<u64> {
        self.records
            .iter()
            .flatten()
            .find(|r| r.phase == Phase::Active)
            .map(|r| r.result.id)
    }
}
