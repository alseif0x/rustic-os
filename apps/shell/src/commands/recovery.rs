// SPDX-License-Identifier: Apache-2.0
use super::*;
use rustic_sdk::files::{Receipt, Retry};
fn token(value: &str) -> Result<Retry, Error> {
    if value.len() != 64 || !value.is_ascii() {
        return Err(Error::Usage);
    }
    let mut bytes = [0; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).map_err(|_| Error::Usage)?;
    }
    Retry::decode(&bytes).map_err(Into::into)
}
fn print_receipt(r: Receipt) {
    output::format(format_args!(
        "committed id={} previous={} version={} bytes={}\r\n",
        r.id, r.previous, r.committed, r.length
    ));
}
pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    match argument(a, 0)? {
        "retry-key" => {
            exact(a, 3)?;
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            let retry = s.files.retry_token(id, number(a, 2)?)?;
            output::text("retry-key=");
            for b in retry.encode() {
                output::format(format_args!("{b:02x}"));
            }
            output::text("\r\n");
        }
        "receipt" => {
            exact(a, 3)?;
            // Numeric object IDs keep owner receipt lookup possible after deletion.
            let id = u32::try_from(number(a, 1)?).map_err(|_| Error::Usage)?;
            print_receipt(s.files.receipt(id, token(argument(a, 2)?)?)?);
        }
        "replace" => {
            exact(a, 5)?;
            let id = s.files.resolve(s.cwd, argument(a, 1)?)?;
            print_receipt(s.files.replace_tracked(
                id,
                number(a, 2)?,
                token(argument(a, 3)?)?,
                argument(a, 4)?.as_bytes(),
            )?);
        }
        "rotate-receipts" => {
            exact(a, 1)?;
            let r = s.service([
                rustic_sdk::abi::supervisor::ROTATE_RECEIPTS,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ])?;
            rustic_sdk::files::Error::parse(r[1] as u8)?;
            output::format(format_args!(
                "retry epoch={} previous receipts expired\r\n",
                r[2]
            ));
        }
        _ => return Err(Error::Unknown),
    }
    Ok(())
}
