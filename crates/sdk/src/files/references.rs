// SPDX-License-Identifier: Apache-2.0
//! Resolve already authorized native objects to stable identities, without inspection rights.
use super::{Client, Error, reference::References};
impl<P: crate::rpc::Progress> Client<P> {
    pub fn references(&mut self, workspace: u32, object: u32) -> Result<References, Error> {
        let reply = self.observation(References::request(workspace, object, self.context)?)?;
        let result = References::decode(&reply)?;
        if result.workspace.root() != workspace || result.resource.object() != object {
            return Err(Error::Protocol);
        }
        Ok(result)
    }
}
