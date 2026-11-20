// SPDX-License-Identifier: Apache-2.0
//! Make owner control available before mounting files. Catalog processes start explicitly.
use super::services::*;
use rustic_sdk::{files::Client, ipc::Endpoint, process, rpc::Rpc, runtime::abi as k};
/// Internal startup discriminator: default V7/manual, V5 fixture, or V7/workspace.
pub fn start(startup: u64) -> Result<State, ()> {
    let (profile, shell_policy, initialize) = match startup {
        0 => (
            FileProfile::V7,
            rustic_supervisor::shell_binding::ShellPolicy::Manual,
            false,
        ),
        1 => (
            FileProfile::V5,
            rustic_supervisor::shell_binding::ShellPolicy::Manual,
            true,
        ),
        2 => (
            FileProfile::V7,
            rustic_supervisor::shell_binding::ShellPolicy::Workspace,
            false,
        ),
        _ => return Err(()),
    };
    let me = process::id().map_err(|_| ())?;
    let shell = spawn(k::SHELL)?;
    let control = connect(me, shell)?;
    let mut state = State {
        profile,
        shell_policy,
        files: 0,
        files_source: FileSource::Embedded,
        deferred_adopted: 0,
        shell,
        admin: Rpc::new(0, 0),
        owner: Client::new(0, 0, 0),
        control: Endpoint::from_bootstrap(control[0]),
        children: [const { None }; rustic_supervisor::topology::CHILD_POOL_SIZE],
        task_result: None,
        admin_drain: false,
        policy: 0,
        takeover: super::takeover::Takeover::new(),
        degraded: true,
        stopping: true,
        work: super::work::Work::new(),
        staged: None,
        #[cfg(feature = "tasks-acceptance")]
        acceptance: super::acceptance::Fixture::default(),
    };
    let job = state.begin_restart(initialize).map_err(|_| ())?;
    call([k::CONSOLE_GRANT, shell, 0, 0, 0, 0, 0, 0])?;
    super::services::start(shell, [0, control[1], job[1]])?;
    Ok(state)
}
