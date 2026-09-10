// SPDX-License-Identifier: Apache-2.0
//! Topological startup: mounted files -> owner policy -> shell. Children stay dormant until provisioned.
use super::services::*;
use rustic_sdk::{files::Client, ipc::Endpoint, process, rpc::Rpc, runtime::abi as k};
pub fn file_service(initialize: bool) -> Result<(u64, Rpc, Client), ()> {
    let me = process::id().map_err(|_| ())?;
    let files = spawn(k::FILES)?;
    let mut admin_tokens = [0; 2];
    let mut data = [0; 2];
    let result = (|| {
        admin_tokens = connect(me, files)?;
        let admin = admin_tokens;
        data = connect(me, files)?;
        let sectors = call([k::DEVICE, 0, 0, 0, 0, 0, 0, 0])?[0];
        let block = call([k::BLOCK_GRANT, files, 7, 0, sectors, 0, 0, 0])?[0];
        super::services::start(files, [block, admin[1], initialize as u64])?;
        let endpoint = Endpoint::from_bootstrap(admin[0]);
        endpoint.wait().map_err(|_| ())?;
        let message = endpoint.receive().map_err(|_| ())?;
        let ready = k::decode(message.payload()).map_err(|_| ())?;
        if message.sender() != files || message.correlation() != 0 || ready[0] != 0 {
            return Err(());
        }
        let mut admin = Rpc::new(endpoint.token(), files);
        let generation = grant(&mut admin, 1, me, data[1], 0, 3, 0)?;
        Ok((files, admin, Client::new(data[0], files, generation)))
    })();
    if result.is_err() {
        stop(files);
        close(me, admin_tokens[0]);
        close(me, data[0]);
    }
    result
}
pub fn start(initialize: bool) -> Result<State, ()> {
    let me = process::id().map_err(|_| ())?;
    call([k::CONSOLE_GRANT, me, 0, 0, 0, 0, 0, 0])?;
    let (files, mut admin, mut owner) = file_service(initialize)?;
    let policy = super::policy::load(&mut owner).unwrap_or(0);
    let shell = spawn(k::SHELL)?;
    let control = connect(me, shell)?;
    let data = connect(files, shell)?;
    let generation = grant(&mut admin, 0, shell, data[0], 0, 3, 0)?;
    call([k::CONSOLE_GRANT, shell, 0, 0, 0, 0, 0, 0])?;
    super::services::start(shell, [data[1], control[1], generation as u64])?;
    Ok(State {
        files,
        shell,
        admin,
        owner,
        control: Endpoint::from_bootstrap(control[0]),
        children: [None, None],
        policy,
    })
}
