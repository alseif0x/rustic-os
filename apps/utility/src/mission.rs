// SPDX-License-Identifier: Apache-2.0
//! One owner-stepped mission; candidate and identity stay in the native client.
mod candidate;
mod control;
use rustic_sdk::{
    abi::{services::Method, supervisor::actor as a},
    files::{Client, Error, admission::AdmissionId, operation::Replacement},
};

#[derive(Default)]
pub(super) struct State {
    // Save before admission, including when acknowledgement is uncertain.
    attempted: Option<Replacement>,
    id: Option<AdmissionId>,
    schedule_attempted: bool,
    cancel_attempted: bool,
}

impl State {
    pub(super) fn execute(&mut self, action: u64, files: &mut Client, scope: u32) -> [u64; 8] {
        let result = match action {
            a::SELECT_GET | a::SELECT_CANCEL => {
                let method = if action == a::SELECT_GET {
                    Method::OperationsGet
                } else {
                    Method::OperationsCancel
                };
                files
                    .select_lifecycle(method)
                    .map(|d| [0, d.availability as u64, method as u64, 1, 2, 0, 0, 0])
            }
            a::MISSION_PREPARE => self.prepare(files, scope),
            a::MISSION_VERIFY => self.verify(files, scope),
            a::MISSION_SCHEDULE => self.schedule(files),
            a::MISSION_INSPECT => self.inspect(files),
            a::MISSION_CANCEL => self.cancel(files),
            _ => Err(Error::Invalid),
        };
        result.unwrap_or_else(|e| [e as u64, 0, 0, 0, 0, 0, 0, 0])
    }
}
