// SPDX-License-Identifier: Apache-2.0
//! Retain one copied native request across service-control polls; never resubmit it.
use core::task::Poll;
use rustic_fs::Error;
use rustic_sdk::block::{Completion, Error as BlockError, Operation, Status};

struct Pending {
    id: u64,
    operation: Operation,
    sector: u64,
    data: [u8; 512],
}

pub trait Requests {
    fn read(&mut self, sector: u64) -> Result<u64, BlockError>;
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<u64, BlockError>;
    fn flush(&mut self) -> Result<u64, BlockError>;
    fn result(&mut self) -> Result<Completion, BlockError>;
}

#[derive(Default)]
pub struct Poller {
    pending: Option<Pending>,
    fenced: bool,
}

impl Poller {
    pub fn ready(&self) -> Result<(), Error> {
        if self.fenced || self.pending.is_some() {
            Err(Error::Uncertain)
        } else {
            Ok(())
        }
    }

    pub fn poll<D: Requests>(
        &mut self,
        device: &mut D,
        operation: Operation,
        sector: u64,
        data: &[u8; 512],
        read_output: Option<&mut [u8; 512]>,
    ) -> Poll<Result<(), Error>> {
        if self.fenced {
            return Poll::Ready(Err(Error::Uncertain));
        }
        self.fenced = true;
        if let Some(pending) = &self.pending {
            if pending.operation != operation
                || pending.sector != sector
                || operation == Operation::Write && &pending.data != data
            {
                return Poll::Ready(Err(Error::Uncertain));
            }
            match device.result() {
                Err(BlockError::WouldBlock) => {
                    self.fenced = false;
                    Poll::Pending
                }
                Ok(done)
                    if done.id == pending.id
                        && done.operation == operation
                        && done.status == Status::Success =>
                {
                    if let Some(output) = read_output {
                        *output = done.data;
                    }
                    self.pending = None;
                    self.fenced = false;
                    Poll::Ready(Ok(()))
                }
                _ => Poll::Ready(Err(Error::Uncertain)),
            }
        } else {
            let id = match operation {
                Operation::Read => device.read(sector),
                Operation::Write => device.write(sector, data),
                Operation::Flush => device.flush(),
            };
            match id {
                Ok(id) => {
                    self.pending = Some(Pending {
                        id,
                        operation,
                        sector,
                        data: *data,
                    });
                    self.fenced = false;
                    Poll::Pending
                }
                Err(_) => Poll::Ready(Err(Error::Uncertain)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_sdk::block::Effect;
    use std::{collections::VecDeque, vec::Vec};

    #[derive(Default)]
    struct Fake {
        attempts: usize,
        submitted: Vec<(Operation, u64, [u8; 512])>,
        completions: VecDeque<Result<Completion, BlockError>>,
        submission_error: Option<BlockError>,
    }

    impl Fake {
        fn submit(
            &mut self,
            operation: Operation,
            sector: u64,
            data: [u8; 512],
        ) -> Result<u64, BlockError> {
            self.attempts += 1;
            if let Some(error) = self.submission_error.take() {
                return Err(error);
            }
            self.submitted.push((operation, sector, data));
            Ok(self.submitted.len() as u64)
        }
    }

    impl Requests for Fake {
        fn read(&mut self, sector: u64) -> Result<u64, BlockError> {
            self.submit(Operation::Read, sector, [0; 512])
        }

        fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<u64, BlockError> {
            self.submit(Operation::Write, sector, *bytes)
        }

        fn flush(&mut self) -> Result<u64, BlockError> {
            self.submit(Operation::Flush, 0, [0; 512])
        }

        fn result(&mut self) -> Result<Completion, BlockError> {
            self.completions
                .pop_front()
                .unwrap_or(Err(BlockError::WouldBlock))
        }
    }

    fn completion(id: u64, operation: Operation, status: Status, byte: u8) -> Completion {
        Completion {
            id,
            operation,
            status,
            effect: if operation == Operation::Read || status != Status::Success {
                Effect::None
            } else {
                Effect::Completed
            },
            data: [byte; 512],
        }
    }

    #[test]
    fn read_retries_same_request_and_copies_only_the_matching_completion() {
        let mut poller = Poller::default();
        let mut device = Fake::default();
        let mut first_destination = [0x11; 512];
        assert_eq!(
            poller.poll(
                &mut device,
                Operation::Read,
                7,
                &[0; 512],
                Some(&mut first_destination)
            ),
            Poll::Pending
        );

        let mut moved_destination = [0x22; 512];
        assert_eq!(
            poller.poll(
                &mut device,
                Operation::Read,
                7,
                &[0; 512],
                Some(&mut moved_destination)
            ),
            Poll::Pending
        );
        assert_eq!(moved_destination, [0x22; 512]);

        device
            .completions
            .push_back(Ok(completion(1, Operation::Read, Status::Success, 0xA5)));
        assert_eq!(
            poller.poll(
                &mut device,
                Operation::Read,
                7,
                &[0; 512],
                Some(&mut moved_destination)
            ),
            Poll::Ready(Ok(()))
        );
        assert_eq!(first_destination, [0x11; 512]);
        assert_eq!(moved_destination, [0xA5; 512]);
        assert_eq!(device.submitted.len(), 1);
        assert_eq!(poller.ready(), Ok(()));
    }

    #[test]
    fn mismatched_request_and_completion_fence_without_copy_or_resubmission() {
        let mut poller = Poller::default();
        let mut device = Fake::default();
        let mut output = [0x33; 512];
        assert_eq!(
            poller.poll(
                &mut device,
                Operation::Read,
                7,
                &[0; 512],
                Some(&mut output)
            ),
            Poll::Pending
        );
        assert_eq!(
            poller.poll(
                &mut device,
                Operation::Read,
                8,
                &[0; 512],
                Some(&mut output)
            ),
            Poll::Ready(Err(Error::Uncertain))
        );
        assert_eq!(
            poller.poll(
                &mut device,
                Operation::Read,
                7,
                &[0; 512],
                Some(&mut output)
            ),
            Poll::Ready(Err(Error::Uncertain))
        );
        assert_eq!(output, [0x33; 512]);
        assert_eq!(device.submitted.len(), 1);
        assert!(device.completions.is_empty());
    }

    #[test]
    fn failed_or_mismatched_completion_is_sticky_and_preserves_destination() {
        for result in [
            completion(99, Operation::Read, Status::Success, 0xA5),
            completion(1, Operation::Read, Status::Io, 0xA5),
            completion(1, Operation::Write, Status::Success, 0xA5),
        ] {
            let mut poller = Poller::default();
            let mut device = Fake::default();
            let mut output = [0x44; 512];
            assert_eq!(
                poller.poll(
                    &mut device,
                    Operation::Read,
                    7,
                    &[0; 512],
                    Some(&mut output)
                ),
                Poll::Pending
            );
            device.completions.push_back(Ok(result));
            assert_eq!(
                poller.poll(
                    &mut device,
                    Operation::Read,
                    7,
                    &[0; 512],
                    Some(&mut output)
                ),
                Poll::Ready(Err(Error::Uncertain))
            );
            assert_eq!(output, [0x44; 512]);
            assert_eq!(
                poller.poll(
                    &mut device,
                    Operation::Read,
                    7,
                    &[0; 512],
                    Some(&mut output)
                ),
                Poll::Ready(Err(Error::Uncertain))
            );
            assert_eq!(device.submitted.len(), 1);
        }
    }

    #[test]
    fn write_retry_must_keep_sector_and_copied_bytes() {
        let mut poller = Poller::default();
        let mut device = Fake::default();
        let bytes = [0x55; 512];
        assert_eq!(
            poller.poll(&mut device, Operation::Write, 9, &bytes, None),
            Poll::Pending
        );
        assert_eq!(
            poller.poll(&mut device, Operation::Write, 9, &[0x56; 512], None),
            Poll::Ready(Err(Error::Uncertain))
        );
        assert_eq!(device.submitted.len(), 1);
    }

    #[test]
    fn successful_write_and_flush_settle_without_resubmission() {
        for (operation, sector, data) in [
            (Operation::Write, 9, [0x55; 512]),
            (Operation::Flush, 0, [0; 512]),
        ] {
            let mut poller = Poller::default();
            let mut device = Fake::default();
            assert_eq!(
                poller.poll(&mut device, operation, sector, &data, None),
                Poll::Pending
            );
            device
                .completions
                .push_back(Ok(completion(1, operation, Status::Success, 0)));
            assert_eq!(
                poller.poll(&mut device, operation, sector, &data, None),
                Poll::Ready(Ok(()))
            );
            assert_eq!(poller.ready(), Ok(()));
            assert_eq!(device.attempts, 1);
        }
    }

    #[test]
    fn submission_and_result_errors_fence_without_a_retry() {
        for operation in [Operation::Read, Operation::Write, Operation::Flush] {
            let mut poller = Poller::default();
            let mut device = Fake {
                submission_error: Some(BlockError::Unavailable),
                ..Fake::default()
            };
            assert_eq!(
                poller.poll(&mut device, operation, 7, &[0; 512], None),
                Poll::Ready(Err(Error::Uncertain))
            );
            assert_eq!(poller.ready(), Err(Error::Uncertain));
            assert_eq!(
                poller.poll(&mut device, operation, 7, &[0; 512], None),
                Poll::Ready(Err(Error::Uncertain))
            );
            assert_eq!(device.attempts, 1);
            assert!(device.submitted.is_empty());
        }

        let mut poller = Poller::default();
        let mut device = Fake::default();
        assert_eq!(
            poller.poll(&mut device, Operation::Read, 7, &[0; 512], None),
            Poll::Pending
        );
        device.completions.push_back(Err(BlockError::Unavailable));
        assert_eq!(
            poller.poll(&mut device, Operation::Read, 7, &[0; 512], None),
            Poll::Ready(Err(Error::Uncertain))
        );
        assert_eq!(poller.ready(), Err(Error::Uncertain));
        assert_eq!(
            poller.poll(&mut device, Operation::Read, 7, &[0; 512], None),
            Poll::Ready(Err(Error::Uncertain))
        );
        assert_eq!(device.attempts, 1);
    }
}
