// SPDX-License-Identifier: Apache-2.0
//! Make owner control available before mounting files. Catalog processes start explicitly.
use super::services::*;
use rustic_sdk::{files::Client, ipc::Endpoint, process, rpc::Rpc, runtime::abi as k};
pub fn start(initialize: bool) -> Result<State, ()> {
    let me = process::id().map_err(|_| ())?;
    let shell = spawn(k::SHELL)?;
    let control = connect(me, shell)?;
    let mut state = State {
        files: 0,
        shell,
        admin: Rpc::new(0, 0),
        owner: Client::new(0, 0, 0),
        control: Endpoint::from_bootstrap(control[0]),
        children: [None, None],
        policy: 0,
        takeover: super::takeover::Takeover::new(),
        degraded: true,
        stopping: true,
        work: super::work::Work::new(),
    };
    let job = state.begin_restart(initialize).map_err(|_| ())?;
    call([k::CONSOLE_GRANT, shell, 0, 0, 0, 0, 0, 0])?;
    super::services::start(shell, [0, control[1], job[1]])?;
    Ok(state)
}
