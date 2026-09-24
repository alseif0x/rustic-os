// SPDX-License-Identifier: Apache-2.0
//! Manual client of V7 profile-2 staged admissions: a streamed deterministic
//! pattern admitted without executing it, and status by profile-2 retry
//! identity. Execution, cancellation and status by ID use the
//! profile-independent admission commands.
use super::*;
use crate::commands::operations::{pattern_byte, replacement};

pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "admit-pattern-v7" => {
            exact(a, 8)?;
            let request = replacement(a)?;
            let seed = u8::try_from(number(a, 6)?).map_err(|_| Error::Usage)?;
            let size = u32::try_from(number(a, 7)?).map_err(|_| Error::Usage)?;
            let started = rustic_sdk::runtime::clock();
            let status = s.files.workspace_admit(request, size, |offset, buffer| {
                for (index, byte) in (offset..).zip(buffer.iter_mut()) {
                    *byte = pattern_byte(seed, index);
                }
                Ok(())
            })?;
            let ticks = rustic_sdk::runtime::clock().saturating_sub(started);
            print(status);
            output::format(format_args!("admit-v7 size={size} ticks={ticks}\r\n"));
        }
        "admission-v7" => {
            exact(a, 4)?;
            print(s.files.admission_retry7(
                argument(a, 1)?.parse()?,
                Retry {
                    epoch: argument(a, 2)?.parse()?,
                    key: argument(a, 3)?.parse()?,
                },
            )?);
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
