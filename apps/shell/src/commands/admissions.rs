// SPDX-License-Identifier: Apache-2.0
//! Manual client of the SDK's explicitly scheduled durable admissions.
use super::*;
use rustic_sdk::files::{
    admission::{State, Status},
    operation::{Replacement, Retry},
};

fn print(status: Status) {
    let state = match status.state {
        State::Admitted => "admitted",
        State::Cancelled => "cancelled",
        State::Committed => "committed",
    };
    output::format(format_args!(
        "admission-v1 id={} service_instance={} state={} terminal={}\r\n",
        status.id, status.service_instance, state, status.terminal
    ));
    if let Some(id) = status.completion() {
        output::format(format_args!("completion={}\r\n", id));
    }
}
pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "enable-admissions" => {
            exact(a, 1)?;
            let r = s.service([
                rustic_sdk::abi::supervisor::ENABLE_ADMISSIONS,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ])?;
            rustic_sdk::files::Error::parse(r[1] as u8)?;
            output::text("Explicit admissions enabled; persistent format v4.\r\n");
        }
        "admit-ref" => {
            exact(a, 7)?;
            let request = Replacement {
                workspace: argument(a, 1)?.parse()?,
                resource: argument(a, 2)?.parse()?,
                expected_version: argument(a, 3)?.parse()?,
                retry: Retry {
                    epoch: argument(a, 4)?.parse()?,
                    key: argument(a, 5)?.parse()?,
                },
            };
            print(s.files.admit_file(request, argument(a, 6)?.as_bytes())?);
        }
        "admission" => {
            let result = if a.len() == 2 {
                s.files.admission_get(argument(a, 1)?.parse()?)?
            } else {
                exact(a, 4)?;
                s.files.admission_retry(
                    argument(a, 1)?.parse()?,
                    Retry {
                        epoch: argument(a, 2)?.parse()?,
                        key: argument(a, 3)?.parse()?,
                    },
                )?
            };
            print(result);
        }
        "execute-admission" | "cancel-admission" => {
            exact(a, 2)?;
            let id = argument(a, 1)?.parse()?;
            print(if argument(a, 0)? == "execute-admission" {
                s.files.admission_execute(id)?
            } else {
                s.files.admission_cancel(id)?
            });
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
