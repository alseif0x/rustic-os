// SPDX-License-Identifier: Apache-2.0
use super::Error;
use rustic_abi::ipc::{ALL, TRANSFER};
const CAPACITY: usize = 16;
const PER_OWNER: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Endpoint {
    pub(super) channel: u64,
    pub(super) side: usize,
}
#[derive(Clone, Copy)]
struct Entry {
    id: u64,
    owner: u64,
    endpoint: Endpoint,
    rights: u8,
}
pub(super) struct Table {
    entries: [Option<Entry>; CAPACITY],
    next: u64,
}

impl Table {
    pub(super) const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            next: 1,
        }
    }
    fn room(&self, owner: u64) -> bool {
        self.entries
            .iter()
            .flatten()
            .filter(|e| e.owner == owner)
            .count()
            < PER_OWNER
    }
    fn index(&self, owner: u64, id: u64) -> Result<usize, Error> {
        self.entries
            .iter()
            .position(|e| e.is_some_and(|e| e.id == id && e.owner == owner))
            .ok_or(Error::Handle)
    }
    pub(super) fn grant(
        &mut self,
        owner: u64,
        endpoint: Endpoint,
        rights: u8,
    ) -> Result<u64, Error> {
        if rights == 0 || rights & !ALL != 0 {
            return Err(Error::Denied);
        }
        if !self.room(owner) {
            return Err(Error::Quota);
        }
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Quota)?;
        let next = self.next.checked_add(1).ok_or(Error::Quota)?;
        let id = self.next;
        self.next = next;
        self.entries[slot] = Some(Entry {
            id,
            owner,
            endpoint,
            rights,
        });
        Ok(id)
    }
    pub(super) fn resolve(&self, owner: u64, id: u64, right: u8) -> Result<Endpoint, Error> {
        let entry = self.entries[self.index(owner, id)?].unwrap();
        if entry.rights & right != right {
            return Err(Error::Denied);
        }
        Ok(entry.endpoint)
    }
    pub(super) fn remove(&mut self, owner: u64, id: u64) -> Result<Endpoint, Error> {
        let slot = self.index(owner, id)?;
        Ok(self.entries[slot].take().unwrap().endpoint)
    }
    pub(super) fn transfer(
        &mut self,
        owner: u64,
        id: u64,
        target: u64,
        rights: u8,
    ) -> Result<u64, Error> {
        let slot = self.index(owner, id)?;
        let old = self.entries[slot].unwrap();
        if old.rights & TRANSFER == 0 || rights == 0 || rights & !old.rights != 0 {
            return Err(Error::Denied);
        }
        if target != owner && !self.room(target) {
            return Err(Error::Quota);
        }
        let next = self.next.checked_add(1).ok_or(Error::Quota)?;
        self.entries[slot] = Some(Entry {
            id: self.next,
            owner: target,
            rights,
            ..old
        });
        self.next = next;
        Ok(next - 1)
    }
    pub(super) fn first(&self, owner: u64) -> Option<u64> {
        self.entries
            .iter()
            .flatten()
            .find(|e| e.owner == owner)
            .map(|e| e.id)
    }
    pub(super) fn count(&self) -> usize {
        self.entries.iter().flatten().count()
    }
}
