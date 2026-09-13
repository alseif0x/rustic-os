// SPDX-License-Identifier: Apache-2.0
//! One complete, immutable file snapshot and indexed task responses.

use rustic_sdk::files::{Client, Error as FileError};
use rustic_tasks_contract::{Document, Error as DocumentError};

pub struct State {
    pub(super) document: Option<Document>,
    pub(super) next: usize,
    pub(super) failure: Option<[u64; 8]>,
    pub(super) preview: Option<rustic_tasks_contract::preview::Summary>,
}

impl State {
    pub const fn new() -> Self {
        Self {
            document: None,
            next: 0,
            failure: None,
            preview: None,
        }
    }

    pub fn list(&mut self, files: &mut Client, scope: u32) -> [u64; 8] {
        if let Some(result) = self.failure {
            return result;
        }
        if self.document.is_some() {
            return rustic_tasks_contract::wire::service(4);
        }
        let result = self.load(files, scope);
        match result {
            Ok(document) => {
                self.document = Some(document);
                self.next = 0;
                self.next_response()
            }
            Err(response) => {
                self.failure = Some(response);
                response
            }
        }
    }

    pub fn next(&mut self) -> [u64; 8] {
        if let Some(result) = self.failure {
            return result;
        }
        if self.document.is_none() {
            return rustic_tasks_contract::wire::service(4);
        }
        self.next_response()
    }

    fn load(&self, files: &mut Client, scope: u32) -> Result<Document, [u64; 8]> {
        let mut bytes = [0; rustic_tasks_contract::MAX_BYTES];
        let info = super::snapshot::read(files, scope, &mut bytes).map_err(service)?;
        Document::parse(&bytes[..info.length]).map_err(document_error)
    }

    pub(super) fn next_response(&mut self) -> [u64; 8] {
        let document = self.document.as_ref().unwrap();
        if let Some(task) = document.get(self.next) {
            self.next += 1;
            rustic_tasks_contract::wire::row(task)
        } else {
            self.preview.map_or_else(
                || rustic_tasks_contract::wire::end(document.len()),
                rustic_tasks_contract::wire::preview_end,
            )
        }
    }
}

fn service(error: FileError) -> [u64; 8] {
    rustic_tasks_contract::wire::service(error as u64)
}

fn document_error(error: DocumentError) -> [u64; 8] {
    match error {
        DocumentError::Invalid => rustic_tasks_contract::wire::invalid(),
        DocumentError::Capacity => rustic_tasks_contract::wire::capacity(),
    }
}
