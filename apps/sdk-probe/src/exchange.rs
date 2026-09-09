// SPDX-License-Identifier: Apache-2.0
use rustic_sdk::{
    Error,
    abi::ipc,
    ipc::{Endpoint, Message},
    process,
};
pub(super) fn run(handle: u64, role: u64, peer: u64) -> Result<(), Error> {
    if role > 1 || peer == 0 || process::id()? == peer {
        return Err(Error::Protocol);
    }
    if Message::new(0, &[0; 65]).err() != Some(Error::Ipc(ipc::Error::Size)) {
        return Err(Error::Protocol);
    }
    if Endpoint::from_bootstrap(0).wait() != Err(Error::Ipc(ipc::Error::Handle)) {
        return Err(Error::Protocol);
    }
    let endpoint = Endpoint::from_bootstrap(handle);
    for sequence in 0u64..4 {
        let payload = sequence.to_le_bytes();
        if role == 0 {
            endpoint.send(&Message::new(sequence, &payload)?)?;
        }
        endpoint.wait()?;
        let received = endpoint.receive()?;
        if received.correlation() != sequence
            || received.payload() != payload
            || received.sender() != peer
        {
            return Err(Error::Protocol);
        }
        if role == 1 {
            endpoint.send(&Message::new(sequence, received.payload())?)?;
        }
    }
    endpoint.close()?;
    process::report(0x53444b)?;
    Ok(())
}
