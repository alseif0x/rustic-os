// SPDX-License-Identifier: Apache-2.0
use super::{PublicationCancel, PublicationPhase};
use crate::{Disk, Error, MAX_FILE, Volume, format::Metadata};

/// Exclusively borrows the live volume and its disk through settlement. Preparing
/// is volatile: it neither reserves a durable operation ID nor acknowledges work.
/// Each advance completes one synchronous disk command; it is not an async driver.
/// Disk implementations must not expose another writer to the same volume.
#[must_use = "drive to settlement or cancel before publication"]
pub struct Publication<'a, D: Disk, T: Copy> {
    volume: &'a mut Volume,
    disk: &'a mut D,
    next: Option<Metadata>,
    data: [u8; MAX_FILE],
    first_sector: u64,
    step: usize,
    phase: PublicationPhase,
    result: T,
}

impl<'a, D: Disk, T: Copy> Publication<'a, D, T> {
    pub(crate) fn new(
        volume: &'a mut Volume,
        disk: &'a mut D,
        next: Metadata,
        first_sector: u64,
        bytes: &[u8],
        result: T,
    ) -> Self {
        let mut data = [0; MAX_FILE];
        data[..bytes.len()].copy_from_slice(bytes);
        // Fence even a forgotten guard. Only known pre-publication cancellation
        // or a successful final flush can restore this live writer.
        volume.poisoned = true;
        Self {
            volume,
            disk,
            next: Some(next),
            data,
            first_sector,
            step: 0,
            phase: PublicationPhase::Preparing,
            result,
        }
    }

    pub(crate) fn replayed(volume: &'a mut Volume, disk: &'a mut D, result: T) -> Self {
        Self {
            volume,
            disk,
            next: None,
            data: [0; MAX_FILE],
            first_sector: 0,
            step: 0,
            phase: PublicationPhase::Committed,
            result,
        }
    }

    pub fn phase(&self) -> PublicationPhase {
        self.phase
    }

    /// No speculative receipt or version is returned before successful settlement.
    pub fn result(&self) -> Option<T> {
        (self.phase == PublicationPhase::Committed).then_some(self.result)
    }

    pub fn advance(&mut self) -> Result<PublicationPhase, Error> {
        match self.phase {
            PublicationPhase::Committed | PublicationPhase::Cancelled => return Ok(self.phase),
            PublicationPhase::Uncertain => return Err(Error::Uncertain),
            _ => (),
        }
        let next = self.next.as_ref().unwrap();
        // Also fence a host Disk implementation that unwinds after submitting I/O.
        self.phase = PublicationPhase::Uncertain;
        let result = match self.step {
            index @ 0..=1 => self.disk.write(
                self.first_sector + index as u64,
                &self.data.as_chunks::<512>().0[index],
            ),
            2 => self.disk.flush(),
            index => next.write_step(self.disk, 1 - self.volume.bank, index - 3),
        };
        if result.is_err() {
            self.phase = PublicationPhase::Uncertain;
            return Err(Error::Uncertain);
        }
        self.step += 1;
        let total = 3 + next.write_steps();
        self.phase = if self.step == total {
            self.volume.metadata = self.next.take().unwrap();
            self.volume.bank = 1 - self.volume.bank;
            self.volume.poisoned = false;
            PublicationPhase::Committed
        } else if self.step == total - 1 {
            PublicationPhase::Settling
        } else if self.step == total - 2 {
            PublicationPhase::ReadyToPublish
        } else {
            PublicationPhase::Preparing
        };
        Ok(self.phase)
    }

    /// All earlier commands have completed. A request after header publication
    /// must still drive the final flush; cancellation cannot roll that write back.
    pub fn cancel(&mut self) -> Result<PublicationCancel, Error> {
        match self.phase {
            PublicationPhase::Preparing
            | PublicationPhase::ReadyToPublish
            | PublicationPhase::Cancelled => {
                self.phase = PublicationPhase::Cancelled;
                self.volume.poisoned = false;
                Ok(PublicationCancel::Cancelled)
            }
            PublicationPhase::Settling | PublicationPhase::Committed => {
                Ok(PublicationCancel::TooLate)
            }
            PublicationPhase::Uncertain => Err(Error::Uncertain),
        }
    }

    pub(crate) fn run(mut self) -> Result<T, Error> {
        while self.phase != PublicationPhase::Committed {
            if self.phase == PublicationPhase::Cancelled {
                return Err(Error::Invalid);
            }
            self.advance()?;
        }
        Ok(self.result)
    }
}

impl<D: Disk, T: Copy> Drop for Publication<'_, D, T> {
    fn drop(&mut self) {
        // Dropping before publication abandons scratch writes with no live effect.
        // Dropping during settlement or after an error leaves the volume fenced.
        if matches!(
            self.phase,
            PublicationPhase::Preparing | PublicationPhase::ReadyToPublish
        ) {
            self.volume.poisoned = false;
        }
    }
}
