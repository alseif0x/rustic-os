// SPDX-License-Identifier: Apache-2.0
use super::{Command, PublicationCancel, PublicationPhase};
use crate::{Disk, Error, MAX_FILE, PollDisk, Volume, format::Metadata};
use core::task::Poll;

/// Exclusively borrows the live volume and its disk through settlement. Preparing
/// is volatile: it neither reserves a durable operation ID nor acknowledges work.
/// advance settles one synchronous command. poll_advance submits or polls one
/// command without blocking, retaining exclusive ownership across Pending.
/// Disk implementations must not expose another writer to the same volume.
#[must_use = "drive to settlement or cancel before publication"]
pub struct Publication<'a, D, T: Copy> {
    volume: &'a mut Volume,
    disk: &'a mut D,
    next: Option<Metadata>,
    data: [u8; MAX_FILE],
    first_sector: u64,
    data_steps: usize,
    step: usize,
    phase: PublicationPhase,
    result: T,
    pending: bool,
    stopping: bool,
}

impl<'a, D, T: Copy> Publication<'a, D, T> {
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
            data_steps: 3,
            step: 0,
            phase: PublicationPhase::Preparing,
            result,
            pending: false,
            stopping: false,
        }
    }

    /// Metadata-only transitions share the same encoder, fence and settlement
    /// rules as file publication, without writing a speculative data extent.
    pub(crate) fn metadata(
        volume: &'a mut Volume,
        disk: &'a mut D,
        mut next: Metadata,
        result: T,
    ) -> Result<Self, Error> {
        next.sequence = volume.sequence().checked_add(1).ok_or(Error::Exhausted)?;
        let mut write = Self::new(volume, disk, next, 0, &[], result);
        write.data_steps = 0;
        Ok(write)
    }

    pub(crate) fn replayed(volume: &'a mut Volume, disk: &'a mut D, result: T) -> Self {
        Self {
            volume,
            disk,
            next: None,
            data: [0; MAX_FILE],
            first_sector: 0,
            data_steps: 0,
            step: 0,
            phase: PublicationPhase::Committed,
            result,
            pending: false,
            stopping: false,
        }
    }

    pub fn phase(&self) -> PublicationPhase {
        self.phase
    }

    /// No speculative receipt or version is returned before successful settlement.
    pub fn result(&self) -> Option<T> {
        (self.phase == PublicationPhase::Committed).then_some(self.result)
    }

    /// True means a submitted command still needs settlement, never cancellation proof.
    pub fn pending(&self) -> bool {
        self.pending
    }

    fn drive(
        &mut self,
        execute: impl FnOnce(&Command, &mut D) -> Poll<Result<(), Error>>,
    ) -> Poll<Result<PublicationPhase, Error>> {
        match self.phase {
            PublicationPhase::Committed | PublicationPhase::Cancelled => {
                return Poll::Ready(Ok(self.phase));
            }
            PublicationPhase::Uncertain => return Poll::Ready(Err(Error::Uncertain)),
            _ => (),
        }
        let next = self.next.as_ref().unwrap();
        let total = self.data_steps + next.write_steps();
        let before = if self.step >= total - 2 {
            PublicationPhase::Settling
        } else {
            self.phase
        };
        let command = if self.step >= self.data_steps {
            next.command(1 - self.volume.bank, self.step - self.data_steps)
                .unwrap()
        } else {
            match self.step {
                index @ 0..=1 => Command::Write(
                    self.first_sector + index as u64,
                    self.data.as_chunks::<512>().0[index],
                ),
                2 => Command::Flush,
                _ => unreachable!(),
            }
        };
        // Fence adapter unwind before crossing the I/O boundary. The header is
        // already too late while its completion is pending, not just after success.
        self.phase = PublicationPhase::Uncertain;
        self.pending = true;
        match execute(&command, self.disk) {
            Poll::Pending => {
                self.phase = before;
                return Poll::Pending;
            }
            Poll::Ready(Err(_)) => return Poll::Ready(Err(Error::Uncertain)),
            Poll::Ready(Ok(())) => self.pending = false,
        }
        self.step += 1;
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
        if self.stopping {
            // Only early cancellation can latch this flag; the one outstanding
            // scratch command has now settled and no header can be submitted.
            self.phase = PublicationPhase::Cancelled;
            self.volume.poisoned = false;
        }
        Poll::Ready(Ok(self.phase))
    }

    /// An outstanding scratch command must drain before cancellation is confirmed.
    /// After header submission continue settlement; cancellation is not rollback.
    pub fn cancel(&mut self) -> Result<PublicationCancel, Error> {
        match self.phase {
            PublicationPhase::Preparing
            | PublicationPhase::ReadyToPublish
            | PublicationPhase::Cancelled => {
                if self.pending {
                    self.stopping = true;
                    return Ok(PublicationCancel::Draining);
                }
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
}

impl<D: PollDisk, T: Copy> Publication<'_, D, T> {
    pub fn poll_advance(&mut self) -> Poll<Result<PublicationPhase, Error>> {
        self.drive(|command, disk| command.poll(disk))
    }
}

impl<D: Disk, T: Copy> Publication<'_, D, T> {
    pub fn advance(&mut self) -> Result<PublicationPhase, Error> {
        // Never turn a pending async command into a second synchronous submission.
        if self.pending {
            return Err(Error::Uncertain);
        }
        match self.drive(|command, disk| Poll::Ready(command.execute(disk))) {
            Poll::Ready(result) => result,
            Poll::Pending => unreachable!(),
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

impl<D, T: Copy> Drop for Publication<'_, D, T> {
    fn drop(&mut self) {
        // Dropping before publication abandons scratch writes with no live effect.
        // Dropping during settlement or after an error leaves the volume fenced.
        if !self.pending
            && matches!(
                self.phase,
                PublicationPhase::Preparing | PublicationPhase::ReadyToPublish
            )
        {
            self.volume.poisoned = false;
        }
    }
}
