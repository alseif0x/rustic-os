// SPDX-License-Identifier: Apache-2.0
//! Capture only bounded retained metadata and scope proofs; never copy file bytes.
use super::{Candidate, ExecutionQueue};
use crate::{Server, admission::scope::Scope, reply};
use rustic_abi::files::Error;

impl ExecutionQueue {
    pub(super) fn refresh(&mut self, server: &Server) -> Result<(), Error> {
        self.candidates.fill(None);
        for (slot, candidate) in self.candidates.iter_mut().enumerate() {
            match server.volume.retained_admission(slot) {
                Ok(Some((subject, admission))) => {
                    *candidate = Some(Candidate {
                        scope: Scope::new(server, subject, &admission)?,
                        subject,
                        status: admission.status,
                    });
                }
                Ok(None) => (),
                Err(rustic_fs::Error::Unsupported) => return Ok(()),
                Err(error) => return Err(reply::error(error)),
            }
        }
        Ok(())
    }
}
