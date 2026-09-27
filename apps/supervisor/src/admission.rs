// SPDX-License-Identifier: Apache-2.0
//! Live resource sampling and admission at the supervisor's allocation boundary.
use super::services::State;
use rustic_sdk::runtime::abi as k;
use rustic_supervisor::topology::{self, Admission, Budget, Usage};

pub(super) fn usage(state: &State) -> Result<Usage, u64> {
    let process = super::services::call([k::INFO, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
    let ipc = super::services::call([k::IPC_INFO, 0, 0, 0, 0, 0, 0, 0]).map_err(|_| 4u64)?;
    let children = state.children.iter().flatten().count();
    let pending_child = state
        .work
        .reserved()
        .filter(|slot| *slot < state.children.len() && state.children[*slot].is_none())
        .is_some();
    let file_children = state
        .children
        .iter()
        .flatten()
        .filter(|child| topology::child_role(child.role) == Some(topology::ChildRole::FileAccess))
        .count();
    let pending_file_child =
        state.work.pending_child_role() == Some(topology::ChildRole::FileAccess);
    let reserved_clients = if state.files != 0 {
        topology::DECLARED.reserved_file_clients as usize
    } else {
        0
    };
    let stage_reservation = usize::from(state.work.staging_pending());

    Ok(Usage {
        processes: (process[4] as usize + stage_reservation).min(u16::MAX as usize) as u16,
        channels: ipc[1].min(u16::MAX as u64) as u16,
        handles: ipc[3].min(u16::MAX as u64) as u16,
        per_owner_handles: ipc[5].min(u16::MAX as u64) as u16,
        file_clients: (reserved_clients + file_children + usize::from(pending_file_child))
            .min(u16::MAX as usize) as u16,
        child_slots: (children + usize::from(pending_child)).min(u16::MAX as usize) as u16,
    })
}

pub(super) fn admit(state: &State, admission: Admission) -> Result<(), u64> {
    let usage = usage(state)?;
    topology::admit(&topology::DECLARED, usage, admission).map_err(Budget::status)
}

pub(super) fn report(state: &State) -> Result<[u64; 8], u64> {
    let usage = usage(state)?;
    let t = &topology::DECLARED;
    Ok([
        0,
        topology::pack_used_limit(usage.processes, t.kernel.processes),
        topology::pack_used_limit(usage.channels, t.kernel.channels),
        topology::pack_used_limit(usage.handles, t.kernel.handles),
        topology::pack_used_limit(usage.per_owner_handles, t.kernel.handles_per_owner),
        topology::pack_used_limit(usage.file_clients, t.file_client_slots),
        topology::pack_used_limit(usage.child_slots, t.child_slots),
        topology::pack_reserve(t),
    ])
}
