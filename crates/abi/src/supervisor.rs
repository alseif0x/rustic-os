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
pub const ENABLE_PREVENTION_REASONS: u64 = 25;
/// Start one bounded read-only tasks listing over a private native child.
pub const TASKS_LIST: u64 = 26;
/// Fetch one cached row from a completed tasks listing.
pub const TASKS_ROW: u64 = 27;
/// Abort a pending or cached tasks listing and reclaim its child.
pub const TASKS_ABORT: u64 = 28;
/// Read-only candidate rows for a task edit; does not submit a mutation.
pub const TASKS_PREVIEW: u64 = 29;
/// Retrieve one bounded byte chunk from a completed task edit candidate.
pub const TASKS_CANDIDATE: u64 = 30;
pub const SESSION: u64 = 8;
pub const HELPER: u64 = 9;
/// Separate native tasks application; it never receives console authority.
pub const TASKS: u64 = 14;
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
    /// Ask the bound service what it implements; the answer grants nothing.
    pub const CAPABILITIES: u64 = 14;
    pub const PROFILE_GET: u64 = 15;
    pub const PROFILE_CANCEL: u64 = 16;
    pub const SELECT_GET: u64 = 17;
    pub const SELECT_CANCEL: u64 = 18;
    pub const MISSION_PREPARE: u64 = 19;
    pub const MISSION_VERIFY: u64 = 20;
    pub const MISSION_SCHEDULE: u64 = 21;
    pub const MISSION_INSPECT: u64 = 22;
    pub const MISSION_CANCEL: u64 = 23;
    /// Modifiers for ADMISSION. These are flags, not action values.
    pub mod flags {
        /// Submit a live stop and exit without decoding its reply. A discarded
        /// acknowledgement is not evidence that the stop was refused.
        pub const DISCARD_REPLY: u64 = 1;
        /// OBSERVE only: explicitly request cause-aware observation profile 2.
        pub const OBSERVE_V2: u64 = 2;
        /// Typed service-v2 projection of one profile-2 observation.
        pub const LIFECYCLE: u64 = 3;
        /// Select the reviewed lifecycle profile before inspection/cancellation.
        pub const NEGOTIATED: u64 = 4;
        /// Use a profile already selected on this same client, without discovery.
        pub const SELECTED: u64 = 5;
    }
}
/// Private requests and responses used only by the native tasks application.
pub mod tasks {
    pub const LIST: u64 = 1;
    pub const NEXT: u64 = 2;
    pub const ROW: u64 = 0;
    pub const END: u64 = 1;
    pub const INVALID: u64 = 2;
    pub const CAPACITY: u64 = 3;
    pub const SERVICE: u64 = 4;
    /// Supervisor status for a fully read but invalid task document.
    pub const INVALID_DOCUMENT: u64 = 7;
    /// Supervisor status for a document outside the bounded task contract.
    pub const CAPACITY_EXCEEDED: u64 = 8;
    /// File-service errors are preserved after this offset in owner job results.
    pub const FILE_ERROR_BASE: u64 = 32;
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
/// Owner-issued diagnostic actor whose durable-operation subject is its own PID.
/// The caller cannot select or impersonate an existing subject.
pub const PRIVATE_ADMISSION_SESSION: u64 = 13;
