// SPDX-License-Identifier: Apache-2.0
//! The second native semantic tasks client: it applies plans, it never makes
//! them.
//!
//! This child never resolves a path, never creates its journal and never reads
//! or writes the object it was granted as `other` beyond handing it to the owner
//! client as the record location. Planning stays in the read-only tasks
//! application; the shell relays the resulting candidate here one bounded step
//! at a time, and this client retains, submits, proves, recovers and forgets it
//! under its own two-scope grant (target `scope`, journal `other`, rights 7).
//!
//! # Steps
//!
//! The owner steps are the `rustic_sdk::abi::supervisor::actor::TASKS_*` actions.
//! Only words 0..4 of a reply are observable through `ACT_STATUS`, so every
//! reply keeps its meaning inside them.
//!
//! | Step | Reply words 0..4 |
//! | --- | --- |
//! | `TASKS_EDIT`, `TASKS_BEGIN`, `TASKS_CHUNK`, `TASKS_FORGET` | `[error, cursor, total, phase, 0]` |
//! | `TASKS_APPLY` | `[error, task_id, journal_key, applied, committed_version]` |
//! | `TASKS_STATUS` | `[error, phase, cursor, total, pending_journal_version]` |
//! | `TASKS_RECOVER` | `[error, recovered, journal_key, task_id, version]` |
//!
//! `TASKS_APPLY` is the one step that reads a request word: word 1 selects the
//! failure cut the submission must take.
//!
//! | Word 1 | Cut |
//! | --- | --- |
//! | 0 | none: the ordinary single submission |
//! | 1 | retain the intent and stop before submitting it |
//! | 2 | submit the replacement and discard its reply |
//! | 3 | retain the intent but lose the acknowledgement of the retention |
//! | 4 | write foreign bytes over the target first, so the submission conflicts |
//!
//! Only `0` exists in an ordinary build. The cuts are compiled in by the
//! `tasks-acceptance` feature alone; without it any other selector is refused as
//! an out-of-phase step (`105`) and changes nothing, exactly as an unknown step
//! is. The supervisor refuses an unimplemented selector before it is delivered.
//!
//! `applied` is `0` for an unchanged document, `1` for a committed and verified
//! effect and `2` for a committed and verified effect whose intent could not be
//! cleared; in that last case `error` still carries the cleanup failure, because
//! the effect is durable but the record is not yet resolved. `recovered` is `1`
//! only when a retained intent was matched to a committed effect.
//!
//! # Phases
//!
//! | Value | Meaning |
//! | --- | --- |
//! | 0 | idle: nothing is held |
//! | 1 | an edit is stored |
//! | 2 | candidate bytes are being collected |
//! | 3 | a validated candidate is ready to apply |
//! | 4 | the last apply concluded and released its candidate |
//!
//! # Error codes
//!
//! `0` is success. A file refusal keeps the numbering the file ABI already uses
//! (1..=31), so a code means the same thing here as in every other reply this
//! application sends. A service or native-application refusal is `64 + code`.
//! The remaining owner-client refusals have fixed codes: `100` document, `102`
//! replacement not enabled, `103` journal, `104` an unresolved intent blocks the
//! mutation. `105` is this client's own refusal: the step is not one the current
//! phase accepts. The owner client's `101` capacity is unreachable here: only
//! its planning relay raises it, and this client never plans. Retention
//! exhaustion is a file-service refusal and arrives as the file ABI's `Full`.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod owner;
pub mod report;
pub mod state;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use owner::run;
