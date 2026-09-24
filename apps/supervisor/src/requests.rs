// SPDX-License-Identifier: Apache-2.0
//! Owner-only shell requests. Process control never accepts arbitrary catalog/subject authority.
use super::services::*;
use rustic_sdk::{abi::supervisor as s, runtime::abi as k};
#[cfg(feature = "tasks-acceptance")]
use rustic_tasks_contract::acceptance as tasks_acceptance;
impl State {
    pub fn request(&mut self, w: [u64; 8]) -> Result<[u64; 8], u64> {
        let end = match w[0] {
            s::INFO
            | s::EXIT
            | s::SERVICES
            | s::RESTART
            | s::ROTATE_RECEIPTS
            | s::IO_STATUS
            | s::ENABLE_OPERATIONS
            | s::ENABLE_ADMISSIONS
            | s::ENABLE_PREVENTION_REASONS => 1,
            s::PROCESS
            | s::KILL
            | s::REAP
            | s::PERMISSIONS
            | s::REVOKE
            | s::REVOCATION
            | s::ACT_STATUS
            | s::STALL_FILES
            | s::JOB_STATUS
            | s::TASKS_LIST
            | s::TASKS_ABORT => 2,
            s::TASKS_ROW => 3,
            s::TASKS_CANDIDATE => 3,
            s::TASKS_OWNER_FORGET | s::TASKS_OWNER_APPLY_CUT => 3,
            s::TASKS_OWNER_BEGIN | s::TASKS_OWNER_CHUNK => 7,
            s::TASKS_PREVIEW | s::TASKS_OWNER_EDIT | s::STAGE_V7 => 8,
            s::HOLD_IO => 3,
            s::RUN => 6,
            // The seventh word carries the deliberate discard flag for a live stop.
            s::ACT_ADMISSION => 7,
            s::HELPER_START => 4,
            s::ACT | s::MOVE_CHECK => 3,
            #[cfg(feature = "tasks-acceptance")]
            tasks_acceptance::ARM
            | tasks_acceptance::STATUS
            | tasks_acceptance::RELEASE
            | tasks_acceptance::RESET => 1,
            _ => return Err(1),
        };
        if w[end..].iter().any(|v| *v != 0) {
            return Err(1);
        }
        self.expire_task_result();
        match w[0] {
            s::INFO => {
                let r = call([k::INFO, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
                // Word 7 is the kernel's heap-page total; it is forwarded as
                // reported, since the shell reply has no word left to shift.
                Ok([0, r[1], r[2], r[3], r[4], r[5], r[6], r[7]])
            }
            s::PROCESS => {
                let r = call([k::PROCESS, w[1], 0, 0, 0, 0, 0, 0]).map_err(|_| 1u64)?;
                Ok([0, r[0], r[1], r[2], r[3], r[4], r[5], r[6]])
            }
            s::RESTART => self.restart(),
            s::ENABLE_OPERATIONS => {
                self.start_admin(s::ENABLE_OPERATIONS, [41, 0, 0, 0, 0, 0, 0, 0])
            }
            s::ENABLE_ADMISSIONS => {
                self.start_admin(s::ENABLE_ADMISSIONS, [42, 0, 0, 0, 0, 0, 0, 0])
            }
            s::ENABLE_PREVENTION_REASONS => {
                self.start_admin(s::ENABLE_PREVENTION_REASONS, [43, 0, 0, 0, 0, 0, 0, 0])
            }
            s::ROTATE_RECEIPTS => self.start_admin(s::ROTATE_RECEIPTS, [36, 0, 0, 0, 0, 0, 0, 0]),
            s::SERVICES => Ok([
                0,
                self.files,
                self.shell,
                self.policy as u64,
                u64::from(self.administrative_ready()),
                0,
                0,
                0,
            ]),
            s::RUN => self.launch(
                w[1],
                u32::try_from(w[2]).map_err(|_| 1u64)?,
                u32::try_from(w[3]).map_err(|_| 1u64)?,
                u8::try_from(w[4]).map_err(|_| 1u64)?,
                w[5],
                0,
            ),
            s::KILL => {
                if !self.children.iter().flatten().any(|c| c.pid == w[1]) && !self.staged_pid(w[1])
                {
                    return Err(2);
                }
                call([k::KILL, w[1], 0, 0, 0, 0, 0, 0]).map_err(|_| 3u64)?;
                Ok([0; 8])
            }
            s::REAP if self.staged_pid(w[1]) => self.reap_staged(w[1]),
            s::REAP => self.reap(w[1]),
            s::STAGE_V7 => self.stage_v7(w),
            s::PERMISSIONS => {
                if w[1] == 0 {
                    return Ok([0, 0, 15, 0, 0, 0, 0, 0]);
                }
                if let Some(facts) = self.staged_facts(w[1]) {
                    return Ok(facts);
                }
                let c = self
                    .children
                    .iter()
                    .flatten()
                    .find(|c| c.pid == w[1])
                    .ok_or(2u64)?;
                Ok([
                    0,
                    c.scope as u64,
                    c.rights as u64,
                    c.generation as u64,
                    c.expires,
                    c.report[0],
                    c.report[1],
                    c.report[2],
                ])
            }
            s::REVOCATION => self.takeover.status(w[1]),
            s::ACT_STATUS => self.actor_status(w[1]),
            s::JOB_STATUS => self.work.status(w[1], self.files),
            s::TASKS_LIST => self.task_list(u32::try_from(w[1]).map_err(|_| 1u64)?),
            s::TASKS_PREVIEW => self.task_preview(
                u32::try_from(w[1]).map_err(|_| 1u64)?,
                rustic_tasks_contract::preview::Edit::decode(w[2..].try_into().map_err(|_| 1u64)?)
                    .ok_or(1u64)?,
            ),
            s::TASKS_ROW => self.task_row(w[1], w[2]),
            s::TASKS_CANDIDATE => self.task_candidate(w[1], w[2]),
            s::TASKS_ABORT => self.abort_task_list(w[1]),
            s::TASKS_OWNER_BEGIN
            | s::TASKS_OWNER_EDIT
            | s::TASKS_OWNER_CHUNK
            | s::TASKS_OWNER_FORGET
            | s::TASKS_OWNER_APPLY_CUT => self.tasks_owner(w),
            #[cfg(feature = "tasks-acceptance")]
            tasks_acceptance::ARM
            | tasks_acceptance::STATUS
            | tasks_acceptance::RELEASE
            | tasks_acceptance::RESET => {
                let request = tasks_acceptance::decode_request(w).ok_or(1u64)?;
                let admin_pending = self.admin.pending();
                self.acceptance.control(request, admin_pending)
            }
            s::STALL_FILES => {
                if w[1] > 1000 {
                    return Err(1);
                }
                self.start_admin(s::STALL_FILES, [39, w[1], 0, 0, 0, 0, 0, 0])
            }
            s::IO_STATUS => {
                let r = call([k::OBSERVATION_STATUS, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
                Ok([0, r[0], r[1], r[2], r[3], r[4], r[5], r[6]])
            }
            s::HOLD_IO => {
                call([k::HOLD_COMPLETION, self.files, w[1], w[2], 0, 0, 0, 0]).map_err(|_| 1u64)?;
                Ok([0; 8])
            }
            s::REVOKE => self.revoke_session(w[1]),
            s::HELPER_START => self.helper(
                w[1],
                u32::try_from(w[2]).map_err(|_| 1u64)?,
                u32::try_from(w[3]).map_err(|_| 1u64)?,
            ),
            s::ACT => self.actor(w[1], w[2]),
            s::ACT_ADMISSION => self.admission_actor(w),
            s::MOVE_CHECK => self.move_check(w[1], w[2]),
            s::EXIT => {
                call([k::SHUTDOWN, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
                Ok([0; 8])
            }
            _ => Err(1),
        }
    }
}
