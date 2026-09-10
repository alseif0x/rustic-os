// SPDX-License-Identifier: Apache-2.0
//! One logical read, assembled from independently authenticated and correlated exchanges.
mod collector;
use super::{Client, Error, Packet, read};
use rustic_abi::files::READ_OPEN;
impl<P: crate::rpc::Progress> Client<P> {
    /// Read one bounded, version-pinned range. Only a completely verified range is
    /// returned; every error clears the whole caller buffer. No implicit retry.
    pub fn read_range(
        &mut self,
        request: read::Request,
        out: &mut [u8],
    ) -> Result<read::Info, Error> {
        let mut collector = collector::Collector::new(request, self.context, out)?;
        collector.open(self.observation(request.packet(READ_OPEN, self.context)?)?)?;
        while let Some(chunk) = collector.next()? {
            collector.chunk(self.observation(chunk)?)?;
        }
        collector.finish()
    }
    // These observations retain the same Rpc peer/correlation stream as legacy
    // file calls, while rejecting noncanonical error responses as well as data.
    pub(super) fn observation(&mut self, mut request: Packet) -> Result<Packet, Error> {
        self.require_binding()?;
        request.context = self.context;
        let response = self
            .rpc
            .exchange(&request.encode())
            .map_err(|error| match error {
                crate::Error::Interrupted => Error::Interrupted,
                crate::Error::Ipc(crate::abi::ipc::Error::Closed) => Error::Closed,
                _ => Error::Protocol,
            })?;
        Packet::decode(response.payload())?.checked_reply(request.op, self.context)
    }
}
