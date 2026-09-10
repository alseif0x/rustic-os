// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, Packet};
use rustic_fs::MAX_FILE;
pub(super) struct Transfer {
    pub(super) client: usize,
    pub(super) logical: Option<rustic_abi::files::operation::Replacement>,
    pub(super) retry: Option<rustic_abi::files::recovery::Retry>,
    pub(super) context: u32,
    pub(super) id: u32,
    pub(super) version: u64,
    pub(super) total: usize,
    pub(super) received: usize,
    pub(super) data: [u8; MAX_FILE],
}
pub(super) struct Transfers {
    slots: [Option<Transfer>; 2],
}
impl Transfers {
    pub(super) const fn new() -> Self {
        Self {
            slots: [const { None }; 2],
        }
    }
    pub(super) fn clear(&mut self, client: usize) {
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|s| s.client == client) {
                *slot = None;
            }
        }
    }
    fn index(&self, client: usize, id: u32, context: u32) -> Result<usize, Error> {
        self.slots
            .iter()
            .position(|s| {
                s.as_ref()
                    .is_some_and(|s| s.client == client && s.id == id && s.context == context)
            })
            .ok_or(Error::NoTransfer)
    }
    pub(super) fn begin(&mut self, client: usize, request: &Packet) -> Result<(), Error> {
        if request.arg as usize > MAX_FILE {
            return Err(Error::Size);
        }
        if self.slots.iter().flatten().any(|s| s.client == client) {
            return Err(Error::Busy);
        }
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Busy)?;
        self.slots[slot] = Some(Transfer {
            client,
            logical: if request.op == rustic_abi::files::REPLACE_OPEN {
                Some(rustic_abi::files::operation::Replacement::decode(request)?)
            } else {
                None
            },
            retry: if request.op == rustic_abi::files::TRACK_BEGIN {
                Some(rustic_abi::files::recovery::Retry::decode(
                    request.payload(),
                )?)
            } else {
                None
            },
            context: request.context,
            id: request.id,
            version: request.version,
            total: request.arg as usize,
            received: 0,
            data: [0; MAX_FILE],
        });
        Ok(())
    }
    pub(super) fn chunk(&mut self, client: usize, request: &Packet) -> Result<(), Error> {
        let index = self.index(client, request.id, request.context)?;
        let slot = self.slots[index].as_mut().unwrap();
        if slot.logical.is_some() != (request.op == rustic_abi::files::REPLACE_CHUNK) {
            return Err(Error::Protocol);
        }
        if request.arg as usize != slot.received
            || request.count == 0
            || slot.received + usize::from(request.count) > slot.total
        {
            return Err(Error::Offset);
        }
        let end = slot.received + usize::from(request.count);
        slot.data[slot.received..end].copy_from_slice(request.payload());
        slot.received = end;
        Ok(())
    }
    pub(super) fn take(&mut self, client: usize, request: &Packet) -> Result<Transfer, Error> {
        let index = self.index(client, request.id, request.context)?;
        if self.slots[index].as_ref().unwrap().received != self.slots[index].as_ref().unwrap().total
        {
            return Err(Error::Offset);
        }
        if self.slots[index].as_ref().unwrap().logical.is_some()
            != (request.op == rustic_abi::files::REPLACE_COMMIT)
        {
            return Err(Error::Protocol);
        }
        Ok(self.slots[index].take().unwrap())
    }
    pub(super) fn logical(
        &self,
        client: usize,
    ) -> Option<rustic_abi::files::operation::Replacement> {
        self.slots
            .iter()
            .flatten()
            .find(|s| s.client == client)
            .and_then(|s| s.logical)
    }
    pub(super) fn tracked(&self, client: usize) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|s| s.client == client && s.retry.is_some())
    }
    pub(super) fn count(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}
