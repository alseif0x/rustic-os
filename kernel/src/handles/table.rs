// SPDX-License-Identifier: Apache-2.0
use super::Error;
#[derive(Clone, Copy)]
struct Entry<T: Copy> {
    id: u64,
    owner: u64,
    object: T,
    rights: u8,
}
pub(crate) struct Table<T: Copy, const CAPACITY: usize, const PER_OWNER: usize> {
    entries: [Option<Entry<T>>; CAPACITY],
    next: u64,
    domain: u64,
    allowed: u8,
    transfer_right: u8,
}

impl<T: Copy, const CAPACITY: usize, const PER_OWNER: usize> Table<T, CAPACITY, PER_OWNER> {
    pub(crate) const fn new(domain: u8, allowed: u8, transfer_right: u8) -> Self {
        Self {
            entries: [None; CAPACITY],
            next: 1,
            domain: (domain as u64) << 56,
            allowed,
            transfer_right,
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
    pub(crate) fn grant(&mut self, owner: u64, object: T, rights: u8) -> Result<u64, Error> {
        if rights == 0 || rights & !self.allowed != 0 {
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
        let next = self
            .next
            .checked_add(1)
            .filter(|next| *next < (1 << 56))
            .ok_or(Error::Quota)?;
        let id = self.domain | self.next;
        self.next = next;
        self.entries[slot] = Some(Entry {
            id,
            owner,
            object,
            rights,
        });
        Ok(id)
    }
    pub(crate) fn resolve(&self, owner: u64, id: u64, right: u8) -> Result<T, Error> {
        let entry = self.entries[self.index(owner, id)?].unwrap();
        if entry.rights & right != right {
            return Err(Error::Denied);
        }
        Ok(entry.object)
    }
    pub(crate) fn remove(&mut self, owner: u64, id: u64) -> Result<T, Error> {
        let slot = self.index(owner, id)?;
        Ok(self.entries[slot].take().unwrap().object)
    }
    pub(crate) fn transfer(
        &mut self,
        owner: u64,
        id: u64,
        target: u64,
        rights: u8,
    ) -> Result<u64, Error> {
        let slot = self.index(owner, id)?;
        let old = self.entries[slot].unwrap();
        if self.transfer_right == 0
            || old.rights & self.transfer_right == 0
            || rights == 0
            || rights & !old.rights != 0
        {
            return Err(Error::Denied);
        }
        if target != owner && !self.room(target) {
            return Err(Error::Quota);
        }
        let next = self
            .next
            .checked_add(1)
            .filter(|next| *next < (1 << 56))
            .ok_or(Error::Quota)?;
        self.entries[slot] = Some(Entry {
            id: self.domain | self.next,
            owner: target,
            rights,
            ..old
        });
        self.next = next;
        Ok(self.domain | (next - 1))
    }
    pub(crate) fn first(&self, owner: u64) -> Option<u64> {
        self.entries
            .iter()
            .flatten()
            .find(|e| e.owner == owner)
            .map(|e| e.id)
    }
    pub(crate) fn count(&self) -> usize {
        self.entries.iter().flatten().count()
    }
}
