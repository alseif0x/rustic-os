// SPDX-License-Identifier: Apache-2.0
//! Nonblocking ownership contract for one write/flush at a time.
use crate::Error;
use core::task::Poll;

/// Each method returns promptly. On Pending the adapter owns exactly one admitted
/// command, including a copy of write bytes. Poll the same command until Ready;
/// it must never resubmit it. Ready(Ok) means the command has settled, with the
/// same flush ordering as Disk. Errors are not rollback. Dropping the caller
/// does not release device-owned buffers or authorize another writer. The adapter
/// must fence unresolved I/O until explicit recovery, including on caller unwind.
pub trait PollDisk {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>>;
    fn poll_flush(&mut self) -> Poll<Result<(), Error>>;
}
