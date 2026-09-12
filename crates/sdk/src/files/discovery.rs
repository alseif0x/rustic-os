// SPDX-License-Identifier: Apache-2.0
//! Ask the bound service what it implements. The answer is never a permission.
use super::Client;
use rustic_abi::files::{Error, Packet, capabilities::Capabilities};

impl<P: crate::rpc::Progress> Client<P> {
    /// One bounded report from the service actually bound to this client. A
    /// method reported available still enforces its own authority on every call.
    pub fn capabilities(&mut self) -> Result<Capabilities, Error> {
        let reply = self.request(Packet::new(rustic_abi::files::CAPABILITIES))?;
        Capabilities::decode(&reply)
    }
}
