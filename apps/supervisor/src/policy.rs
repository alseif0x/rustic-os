// SPDX-License-Identifier: Apache-2.0
//! Persistent fail-closed owner policy. Runtime grants never survive a boot.
use rustic_sdk::files::{Client, Error};
const CONTENT: &[u8] = b"rustic-owner-v1\nhelpers=explicit\n";
pub fn load(files: &mut Client) -> Result<u32, ()> {
    let node = match files.lookup(3, "owner-policy") {
        Ok(node) => node,
        Err(Error::NotFound) => {
            let node = files.create(3, "owner-policy", false).map_err(|_| ())?;
            files
                .replace(node.id, node.version, CONTENT)
                .map_err(|_| ())?
        }
        Err(_) => return Err(()),
    };
    let mut bytes = [0; 1024];
    let n = files.read(node.id, &mut bytes).map_err(|_| ())?;
    if &bytes[..n] != CONTENT {
        return Err(());
    }
    Ok(node.id)
}
