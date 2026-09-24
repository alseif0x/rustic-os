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
/// Announce the planned candidate to a persistent tasks-owner child.
/// Words: `[31, pid, total, count, version, task_id, changed, 0]`, where the
/// four summary words are those of `rustic_tasks_contract::preview::Summary`.
pub const TASKS_OWNER_BEGIN: u64 = 31;
/// Supply the intended edit to a persistent tasks-owner child.
/// Words: `[32, pid, e0, e1, e2, e3, e4, e5]`, the six `preview::Edit` words.
pub const TASKS_OWNER_EDIT: u64 = 32;
/// Feed one bounded candidate chunk to a persistent tasks-owner child.
/// Words: `[33, pid, length, b0, b1, b2, b3, 0]`; the offset is implicit in the
/// order the chunks are submitted.
pub const TASKS_OWNER_CHUNK: u64 = 33;
/// Discard the recovery evidence recorded under one intent key.
/// Words: `[34, pid, key, 0, 0, 0, 0, 0]`.
pub const TASKS_OWNER_FORGET: u64 = 34;
/// Apply the collected candidate under an explicit failure cut.
/// Words: `[35, pid, cut, 0, 0, 0, 0, 0]`. The cut selects where the single
/// submission fails: `0` none, `1` retained but not submitted, `2` submitted
/// with the reply discarded, `3` retained with the retention acknowledgement
/// lost, `4` a foreign write over the target before the submission.
///
/// The request identifier exists in every build so that its number is never
/// reused, but only an explicit acceptance build implements the cuts: elsewhere
/// the supervisor refuses the whole request as invalid, exactly like an unknown
/// one. An ordinary apply needs no cut and stays the [`ACT`] verb
/// [`actor::TASKS_APPLY`].
pub const TASKS_OWNER_APPLY_CUT: u64 = 35;
/// Stage one ELF and its 128-byte manifest from the read-only V7 file service as
/// a single owner-pinned version pair, producing a dormant child that is never
/// started by this request. Accepted only in the V7 file profile.
///
/// Words: `[36, lineage_lo, lineage_hi, workspace_root, elf, manifest,
/// elf_version, manifest_version]`, where the lineage words are the two
/// little-endian halves of the 16-byte workspace lineage, `elf` and `manifest`
/// are resource objects of that workspace and both versions are the exact
/// `v_...` values the owner observed. The reply is a job; its completed result is
/// `[0, pid, elf_version, manifest_version]` or one [`stage`] refusal status.
pub const STAGE_V7: u64 = 36;
/// Refusal statuses of a completed [`STAGE_V7`] job, beside the generic owner
/// statuses `1` invalid, `2` denied, `3` busy and `4` service failure.
pub mod stage {
    /// The pair observations disagreed: lineage, resource, version, retry epoch,
    /// offset, size or EOF differed from the pinned pair, or the manifest was not
    /// exactly one 128-byte range.
    pub const PAIR_MISMATCH: u64 = 9;
    /// The ELF size is outside the kernel staging bounds.
    pub const IMAGE_SIZE: u64 = 10;
    /// A file-service refusal `e` is reported as `FILE_ERROR_BASE + e`.
    pub const FILE_ERROR_BASE: u64 = 32;
    /// A kernel staging refusal with runtime error index `e` is reported as
    /// `KERNEL_ERROR_BASE + e`.
    pub const KERNEL_ERROR_BASE: u64 = 64;
}
/// Start the one child staged by [`STAGE_V7`] under the supervisor's
/// control-only storage topology: a single private control channel carries the
/// role message and the child's report. No file endpoint, file-service peer,
/// block grant or console is issued. The supervisor decides which manifest
/// identity and which roles qualify; the owner only names the child and a role.
///
/// Words: `[37, pid, role, 0, 0, 0, 0, 0]`, where `pid` is the staged child and
/// `role` is one of the utility roles of this module. The reply is immediate:
/// `[0, pid, role, 0, 0, 0, 0, 0]` or a [`launch`] refusal. A refused start
/// leaves the child dormant with no endpoint; it can still be killed and reaped.
pub const START_STAGED: u64 = 37;
/// Refusal statuses of [`START_STAGED`], beside the generic owner statuses `1`
/// invalid, `2` denied (no such staged child) and `4` service failure.
pub mod launch {
    /// The staged manifest identity is not one the supervisor starts from storage.
    pub const IDENTITY: u64 = 11;
    /// The role needs authority the control-only topology does not issue.
    pub const ROLE: u64 = 12;
    /// The manifest does not request the features the issued channel implies.
    pub const FEATURES: u64 = 13;
    /// The staged child was already started once.
    pub const STARTED: u64 = 14;
    /// A kernel refusal of the control channel or of the start, with runtime
    /// error index `e`, is reported as `KERNEL_ERROR_BASE + e`.
    pub const KERNEL_ERROR_BASE: u64 = 64;
}
pub const SESSION: u64 = 8;
pub const HELPER: u64 = 9;
/// Separate native tasks application; it never receives console authority.
pub const TASKS: u64 = 14;
/// Native tasks client holding owner-equivalent authority over exactly two file
/// objects: the target task document (`scope`) and its own journal record
/// (`other`). It is the only role whose grant carries a second object scope.
///
/// `other` must be a live file above identifier `1` and outside `scope`. It is
/// also this client's recovery subject, so its retained operations and receipts
/// share no namespace with the shell's owner client (subject `1`), and a
/// relaunch on the same journal recovers under the same identity.
pub const TASKS_OWNER: u64 = 15;
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
    /// Owner-stepped actions of the persistent tasks-owner child. They are
    /// accepted only for role [`super::TASKS_OWNER`], and that role answers no
    /// other action: a tasks child never serves the read or mission verbs.
    ///
    /// Announce the planned candidate together with the preview summary it was
    /// derived from: `[24, total, count, version, task_id, changed, 0, 0]`. The
    /// four summary words are those of `rustic_tasks_contract::preview::Summary`,
    /// whose own words 4..6 are always zero and are therefore not carried.
    pub const TASKS_BEGIN: u64 = 24;
    /// Supply the intended edit: `[25, e0, e1, e2, e3, e4, e5, 0]`, the six
    /// words of `rustic_tasks_contract::preview::Edit`.
    pub const TASKS_EDIT: u64 = 25;
    /// Supply the next candidate bytes: `[26, length, b0, b1, b2, b3, 0, 0]`,
    /// up to 32 bytes packed as in `rustic_tasks_contract::candidate`. The
    /// offset is implicit: chunks are appended in submission order.
    pub const TASKS_CHUNK: u64 = 26;
    /// Apply the accumulated candidate: `[27, cut, 0, 0, 0, 0, 0, 0]`. The cut
    /// is `0` for the ordinary single submission; the other values exist only
    /// in an acceptance build and are documented on
    /// [`super::TASKS_OWNER_APPLY_CUT`].
    pub const TASKS_APPLY: u64 = 27;
    /// Report the child's own view of the intent: `[28, 0, 0, 0, 0, 0, 0, 0]`.
    pub const TASKS_STATUS: u64 = 28;
    /// Resolve a retained intent from its journal: `[29, 0, 0, 0, 0, 0, 0, 0]`.
    pub const TASKS_RECOVER: u64 = 29;
    /// Discard the recovery evidence of one intent key: `[30, key, 0, ..]`.
    pub const TASKS_FORGET: u64 = 30;
    /// Grow the child's own heap to the per-process page budget and release it
    /// again: `[31, 0, 0, 0, 0, 0, 0, 0]`. Like the other actions above it is
    /// admitted only for role [`super::TASKS_OWNER`] children.
    ///
    /// It carries no candidate and touches no file: the reply is
    /// `[error, peak_pages, full_observed, pages_after_release, limit]`, where
    /// `full_observed` is `1` when a growth was refused with the kernel's
    /// `Full`, `pages_after_release` is the whole process's mapped pages after
    /// the heap was dropped, and `error` is `0` only when the budget was reached
    /// and everything mapped for it was released. Otherwise `error` is `106`,
    /// this client's memory refusal.
    pub const TASKS_HEAP_STRESS: u64 = 31;
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
