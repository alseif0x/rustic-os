// SPDX-License-Identifier: Apache-2.0
//! Only the private bootstrap channel invokes these administrative operations.
use rustic_file_service::{Grant, Server};
use rustic_sdk::abi::files::{Error, GRANT, REVOKE, STATUS};
pub fn dispatch(server: &mut Server, w: [u64; 8]) -> [u64; 8] {
    let mut r = [0; 8];
    let result = (|| {
        if w[7] != 0 {
            return Err(Error::Protocol);
        }
        let slot = usize::try_from(w[1]).map_err(|_| Error::Invalid)?;
        match w[0] {
            x if x == GRANT as u64 => {
                r[1] = server.grant(
                    slot,
                    Grant {
                        peer: w[2],
                        endpoint: w[3],
                        scope: u32::try_from(w[4]).map_err(|_| Error::Invalid)?,
                        rights: u8::try_from(w[5]).map_err(|_| Error::Invalid)?,
                        generation: 0,
                        expires: w[6],
                    },
                )? as u64;
            }
            x if x == REVOKE as u64 && w[2..].iter().all(|x| *x == 0) => server.revoke(slot)?,
            35 if slot < 4 && w[2..].iter().all(|x| *x == 0) => server.detach(slot),
            x if x == STATUS as u64 && w[1..].iter().all(|x| *x == 0) => {
                r[1] = server.pending() as u64;
            }
            _ => return Err(Error::Protocol),
        }
        Ok(())
    })();
    if let Err(error) = result {
        r[0] = error as u64;
    }
    r
}
