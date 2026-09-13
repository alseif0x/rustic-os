// SPDX-License-Identifier: Apache-2.0
//! Apply domain planning to one verified source snapshot, without write authority.
use rustic_sdk::files::{Client, Error as FileError};
use rustic_tasks::{Command, Error};
use rustic_tasks_contract::{
    Document,
    preview::{Edit, Summary},
    wire,
};

impl super::read::State {
    pub(super) fn preview(&mut self, files: &mut Client, scope: u32, edit: Edit) -> [u64; 8] {
        if let Some(failure) = self.failure {
            return failure;
        }
        if self.document.is_some() {
            return wire::service(FileError::Busy as u64);
        }
        let result = (|| {
            let mut original = [0; rustic_tasks_contract::MAX_BYTES];
            let info = super::snapshot::read(files, scope, &mut original)
                .map_err(|e| wire::service(e as u64))?;
            let command = match &edit {
                Edit::Add { title, length } => Command::Add(&title[..usize::from(*length)]),
                Edit::Done { id } => Command::Done(*id),
            };
            let plan = rustic_tasks::plan(&original[..info.length], command).map_err(failure)?;
            let document = Document::parse(plan.bytes())
                .map_err(|_| wire::service(FileError::Protocol as u64))?;
            self.preview = Some(Summary {
                count: document.len() as u32,
                version: info.version.value(),
                task_id: plan.task_id(),
                changed: plan.changed(),
            });
            self.document = Some(document);
            self.next = 0;
            Ok::<(), [u64; 8]>(())
        })();
        match result {
            Ok(()) => self.next_response(),
            Err(response) => {
                self.failure = Some(response);
                response
            }
        }
    }
}

fn failure(error: Error) -> [u64; 8] {
    match error {
        Error::InvalidDocument => wire::invalid(),
        Error::Capacity => wire::capacity(),
        Error::InvalidTitle | Error::InvalidId => wire::service(FileError::Invalid as u64),
        Error::TaskNotFound => wire::service(FileError::NotFound as u64),
        Error::IdExhausted => wire::service(FileError::Exhausted as u64),
    }
}
