// SPDX-License-Identifier: Apache-2.0
//! Shared volatile transfer framing; publication profiles remain distinct.
use super::Client;
use rustic_abi::files::{operation::Replacement, *};
impl<P: crate::rpc::Progress> Client<P> {
    /// Staging is volatile and has no file effect or durable operation acknowledgement.
    pub(super) fn stage_profile(
        &mut self,
        request: Replacement,
        bytes: &[u8],
        admitted: bool,
    ) -> Result<(), Error> {
        let mut first = request.packet(bytes.len(), self.context)?;
        if admitted {
            first.op = admission::OPEN;
        }
        let mut opened = false;
        let stage = (|| {
            let ack = self.operation_exchange(first)?;
            if ack.id != 0 || ack.arg != 0 || ack.version != 0 || ack.count != 0 {
                return Err(Error::Protocol);
            }
            opened = true;
            for (index, chunk) in bytes.chunks(DATA).enumerate() {
                let mut p = Packet::new(if admitted {
                    admission::CHUNK
                } else {
                    REPLACE_CHUNK
                });
                p.id = request.resource.object();
                p.arg = (index * DATA) as u32;
                p.count = chunk.len() as u8;
                p.data[..chunk.len()].copy_from_slice(chunk);
                let ack = self.operation_exchange(p)?;
                if ack.id != 0 || ack.arg != 0 || ack.version != 0 || ack.count != 0 {
                    return Err(Error::Protocol);
                }
            }
            Ok(())
        })();
        if opened && stage.is_err() {
            let mut p = Packet::new(if admitted {
                admission::ABORT
            } else {
                REPLACE_ABORT
            });
            p.id = request.resource.object();
            let _ = self.operation_exchange(p);
        }
        stage
    }
}
