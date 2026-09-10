// SPDX-License-Identifier: Apache-2.0
//! Resolve monotonic object identities within a selected live directory.
use crate::{Error, Kind, Node, Volume};

impl Volume {
    /// Membership is a storage fact, not authorization. The service must check
    /// the caller's grant before exposing this result or an existence error.
    pub fn resolve(&self, workspace: u32, id: u32) -> Result<Node, Error> {
        let directory = self.stat(workspace)?;
        if directory.kind != Kind::Directory {
            return Err(Error::NotDirectory);
        }
        let node = self.stat(id)?;
        if !self.within(id, workspace) {
            return Err(Error::NotFound);
        }
        Ok(node)
    }
}
