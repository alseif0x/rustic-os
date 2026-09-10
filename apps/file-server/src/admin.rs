// SPDX-License-Identifier: Apache-2.0
//! Only the private bootstrap channel invokes these administrative operations.
use rustic_file_service::{Grant, Server};
use rustic_sdk::abi::files::{Error, GRANT, REVOKE, STATUS};
pub fn dispatch(
    server: &mut Server,
    disk: &mut impl rustic_fs::Disk,
    w: [u64; 8],
    now: u64,
) -> [u64; 8] {
    let mut r = [0; 8];
    let result = (|| {
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
                        subject: w[7],
                    },
                )? as u64;
            }
            x if x == REVOKE as u64 && w[2..].iter().all(|x| *x == 0) => {
                let before = server.pending();
                r[1] = server.revoke(slot)? as u64;
                r[2] = (before - server.pending()) as u64;
                r[3] = server.volume.stat(1).is_err() as u64;
                r[4] = server.volume.sequence();
            }
            37 => {
                r[1] = server.derive(
                    slot,
                    u32::try_from(w[2]).map_err(|_| Error::Invalid)?,
                    Grant {
                        peer: w[3],
                        endpoint: w[4],
                        scope: u32::try_from(w[5]).map_err(|_| Error::Invalid)?,
                        rights: u8::try_from(w[6]).map_err(|_| Error::Invalid)?,
                        expires: w[7],
                        generation: 0,
                        subject: 0,
                    },
                    now,
                )? as u64;
            }
            35 if slot < 4 && w[2..].iter().all(|x| *x == 0) => server.detach(slot),
            x if x == STATUS as u64 && w[1..].iter().all(|x| *x == 0) => {
                r[1] = server.pending() as u64;
            }
            36 if w[1..].iter().all(|x| *x == 0) => {
                r[1] = server.volume.advance_epoch(disk).map_err(|e| match e {
                    rustic_fs::Error::Uncertain => Error::Uncertain,
                    rustic_fs::Error::Unsupported => Error::Unsupported,
                    rustic_fs::Error::Exhausted => Error::Exhausted,
                    _ => Error::Io,
                })?;
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
