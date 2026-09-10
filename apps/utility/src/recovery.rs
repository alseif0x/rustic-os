// SPDX-License-Identifier: Apache-2.0
//! Explicit owner-launched fault fixture: drop a real reply without decoding its result.
use rustic_sdk::{
    abi::files::{COMMIT, Error, Packet},
    files::Client,
    ipc::{Endpoint, Message},
};
pub fn discard_reply(files: &mut Client, id: u32, other: u32) -> [u64; 8] {
    let result = (|| {
        if !matches!(files.stat(other), Err(Error::Denied)) {
            return Err(Error::Protocol);
        }
        let retry = files.retry_token(id, 77)?;
        let version = files.stat(id)?.version;
        files.stage_tracked(id, version, retry, b"reply deliberately unobserved")?;
        let mut p = Packet::new(COMMIT);
        p.id = id;
        p.context = files.context;
        let endpoint = Endpoint::from_bootstrap(files.token());
        endpoint
            .send(&Message::new(u64::MAX, &p.encode()).map_err(|_| Error::Protocol)?)
            .map_err(|_| Error::Uncertain)?;
        // Wait for readiness only. Process exit closes the handle and discards the queued reply.
        // The owner must inspect the durable result; readiness itself is not success evidence.
        endpoint.wait().map_err(|_| Error::Uncertain)?;
        Ok(())
    })();
    [result.err().map_or(0, |e| e as u64), 0, 17, 0, 0, 0, 0, 0]
}
