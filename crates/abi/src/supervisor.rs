// SPDX-License-Identifier: Apache-2.0
//! Owner-shell service protocol, on an explicitly provisioned private IPC channel.
// Replies: word 0 status (0 success, 1 invalid, 2 denied, 3 busy, 4 service failure),
// words 1..7 operation-specific values. No caller-selected subject identity.
pub const INFO: u64 = 1;
pub const PROCESS: u64 = 2;
pub const RUN: u64 = 3;
pub const KILL: u64 = 4;
pub const REAP: u64 = 5;
pub const PERMISSIONS: u64 = 6;
pub const REVOKE: u64 = 7;
pub const EXIT: u64 = 8;
pub const SERVICES: u64 = 9;
pub const GRANT: u64 = 10;
pub const RESTART: u64 = 11;
pub const ROTATE_RECEIPTS: u64 = 12;
pub const HELPER_START: u64 = 13;
pub const ACT: u64 = 14;
pub const MOVE_CHECK: u64 = 15;
pub const ACT_STATUS: u64 = 16;
pub const REVOCATION: u64 = 17;
pub const STALL_FILES: u64 = 18;
pub const JOB_STATUS: u64 = 19;
pub const HOLD_IO: u64 = 20;
pub const IO_STATUS: u64 = 21;
pub const ENABLE_OPERATIONS: u64 = 22;
pub const ENABLE_ADMISSIONS: u64 = 23;
pub const ACT_ADMISSION: u64 = 24;
pub const SESSION: u64 = 8;
pub const HELPER: u64 = 9;
/// Deterministic native actor commands; the owner supplies no arbitrary program.
pub mod actor {
    pub const READ: u64 = 1;
    pub const STAGE: u64 = 2;
    pub const COMMIT: u64 = 3;
    pub const FLOOD: u64 = 4;
    pub const DRAIN: u64 = 5;
    pub const MOVED: u64 = 6;
    pub const STALE: u64 = 7;
    pub const API_READ: u64 = 8;
    pub const READ_OPEN: u64 = 9;
    pub const READ_NEXT: u64 = 10;
    pub const FILL: u64 = 11;
    pub const OPERATION_GET: u64 = 12;
    pub const ADMISSION: u64 = 13;
    /// Modifiers for ADMISSION. These are flags, not action values.
    pub mod flags {
        /// Submit a live stop and exit without decoding its reply. A discarded
        /// acknowledgement is not evidence that the stop was refused.
        pub const DISCARD_REPLY: u64 = 1;
    }
}
pub const SPIN: u64 = 1;
pub const FAULT: u64 = 2;
pub const READ: u64 = 3;
pub const PROBE: u64 = 4;
pub const FINISH: u64 = 5;
pub const WATCH: u64 = 6;
pub const LOST_REPLY: u64 = 7;
pub const LOST_OPERATION: u64 = 10;
pub const LOST_ADMISSION: u64 = 11;
/// Explicit owner-issued diagnostic session; requested file rights remain scoped.
pub const ADMISSION_SESSION: u64 = 12;
