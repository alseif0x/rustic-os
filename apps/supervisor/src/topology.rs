// SPDX-License-Identifier: Apache-2.0
//! Declared, measured admission budgets for the native service topology.
use rustic_sdk::abi::supervisor as s;

pub const CHILD_POOL_SIZE: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceUse {
    pub processes: u16,
    pub channels: u16,
    pub handles: u16,
    pub per_owner_handles: u16,
    pub file_clients: u16,
    pub child_slots: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelLimits {
    pub processes: u16,
    pub channels: u16,
    pub handles: u16,
    pub handles_per_owner: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Topology {
    pub kernel: KernelLimits,
    pub file_client_slots: u16,
    pub reserved_file_clients: u16,
    pub child_slots: u16,
    pub base_system: ResourceUse,
    pub tasks_if_launched: ResourceUse,
    pub recovery_reserve: ResourceUse,
    pub control_only_child: ResourceUse,
    pub file_access_child: ResourceUse,
    pub staged_child: ResourceUse,
    pub staged_start: ResourceUse,
    pub adoption: ResourceUse,
}

pub const DECLARED: Topology = Topology {
    kernel: KernelLimits {
        processes: 16,
        channels: 24,
        handles: 64,
        handles_per_owner: 24,
    },
    file_client_slots: 4,
    reserved_file_clients: 2,
    child_slots: CHILD_POOL_SIZE as u16,
    base_system: ResourceUse {
        processes: 3, // supervisor, shell, files
        channels: 4,  // shell-control, admin, owner, shell-files
        handles: 8,
        per_owner_handles: 3,
        file_clients: 2,
        child_slots: 0,
    },
    tasks_if_launched: ResourceUse {
        processes: 1,
        channels: 2,
        handles: 4,
        per_owner_handles: 2,
        file_clients: 1,
        child_slots: 1,
    },
    recovery_reserve: ResourceUse {
        processes: 1,
        channels: 3,
        handles: 6,
        per_owner_handles: 6,
        file_clients: 0,
        child_slots: 0,
    },
    control_only_child: ResourceUse {
        processes: 1,
        channels: 1,
        handles: 2,
        per_owner_handles: 1,
        file_clients: 0,
        child_slots: 1,
    },
    file_access_child: ResourceUse {
        processes: 1,
        channels: 2,
        handles: 4,
        per_owner_handles: 2,
        file_clients: 1,
        child_slots: 1,
    },
    staged_child: ResourceUse {
        processes: 1,
        channels: 0,
        handles: 0,
        per_owner_handles: 0,
        file_clients: 0,
        child_slots: 0,
    },
    staged_start: ResourceUse {
        processes: 0,
        channels: 1,
        handles: 2,
        per_owner_handles: 1,
        file_clients: 0,
        child_slots: 0,
    },
    // Adoption replaces the current file service and its two reserved clients.
    // Its three channels and six handles consume recovery headroom after existing
    // utility clients have been withdrawn by the transition.
    adoption: ResourceUse {
        processes: 0, // the dormant staged process is already in the live count
        channels: 3,
        handles: 6,
        per_owner_handles: 6,
        file_clients: 0,
        child_slots: 0,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    ShellControl,
    Admin,
    Owner,
    ShellFiles,
}

pub const BASE_CHANNELS: [Channel; 4] = [
    Channel::ShellControl,
    Channel::Admin,
    Channel::Owner,
    Channel::ShellFiles,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub processes: u16,
    pub channels: u16,
    pub handles: u16,
    pub per_owner_handles: u16,
    /// Includes the two reserved shell and supervisor-owner clients.
    pub file_clients: u16,
    pub child_slots: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildRole {
    ControlOnly,
    FileAccess,
}

pub fn child_role(role: u64) -> Option<ChildRole> {
    match role {
        s::SPIN | s::FAULT | s::FINISH => Some(ChildRole::ControlOnly),
        s::READ
        | s::PROBE
        | s::WATCH
        | s::LOST_REPLY
        | s::LOST_OPERATION
        | s::LOST_ADMISSION
        | s::ADMISSION_SESSION
        | s::PRIVATE_ADMISSION_SESSION
        | s::SESSION
        | s::HELPER
        | s::TASKS
        | s::TASKS_OWNER => Some(ChildRole::FileAccess),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Child(ChildRole),
    Stage,
    StartStaged,
    AdoptFiles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Budget {
    Processes,
    Channels,
    Handles,
    PerOwnerHandles,
    FileClients,
    ChildSlots,
}

impl Budget {
    pub const fn status(self) -> u64 {
        use rustic_sdk::abi::supervisor::capacity;
        match self {
            Self::Processes => capacity::PROCESSES,
            Self::Channels => capacity::CHANNELS,
            Self::Handles => capacity::HANDLES,
            Self::PerOwnerHandles => capacity::OWNER_HANDLES,
            Self::FileClients => capacity::FILE_CLIENTS,
            Self::ChildSlots => capacity::CHILD_SLOTS,
        }
    }
}

pub const fn cost(topology: &Topology, admission: Admission) -> ResourceUse {
    match admission {
        Admission::Child(ChildRole::ControlOnly) => topology.control_only_child,
        Admission::Child(ChildRole::FileAccess) => topology.file_access_child,
        Admission::Stage => topology.staged_child,
        Admission::StartStaged => topology.staged_start,
        Admission::AdoptFiles => topology.adoption,
    }
}

pub fn admit(topology: &Topology, usage: Usage, admission: Admission) -> Result<(), Budget> {
    let cost = cost(topology, admission);
    let recovery = admission == Admission::AdoptFiles;
    let reserve = if recovery {
        ResourceUse {
            processes: 0,
            channels: 0,
            handles: 0,
            per_owner_handles: 0,
            file_clients: 0,
            child_slots: 0,
        }
    } else {
        topology.recovery_reserve
    };
    if exceeds(
        usage.processes,
        cost.processes,
        topology.kernel.processes - reserve.processes,
    ) {
        return Err(Budget::Processes);
    }
    if exceeds(
        usage.channels,
        cost.channels,
        topology.kernel.channels - reserve.channels,
    ) {
        return Err(Budget::Channels);
    }
    if exceeds(
        usage.handles,
        cost.handles,
        topology.kernel.handles - reserve.handles,
    ) {
        return Err(Budget::Handles);
    }
    if exceeds(
        usage.per_owner_handles,
        cost.per_owner_handles,
        topology.kernel.handles_per_owner - reserve.per_owner_handles,
    ) {
        return Err(Budget::PerOwnerHandles);
    }
    if exceeds(
        usage.file_clients,
        cost.file_clients,
        topology.file_client_slots,
    ) {
        return Err(Budget::FileClients);
    }
    if exceeds(usage.child_slots, cost.child_slots, topology.child_slots) {
        return Err(Budget::ChildSlots);
    }
    Ok(())
}

fn exceeds(used: u16, cost: u16, limit: u16) -> bool {
    u32::from(used) + u32::from(cost) > u32::from(limit)
}

pub fn application_available(limit: u16, reserved: u16, used: u16) -> u16 {
    limit.saturating_sub(reserved).saturating_sub(used)
}

pub const fn pack_used_limit(used: u16, limit: u16) -> u64 {
    (used as u64) | ((limit as u64) << 32)
}

pub const fn unpack_used_limit(word: u64) -> (u16, u16) {
    (word as u16, (word >> 32) as u16)
}

pub const fn pack_reserve(topology: &Topology) -> u64 {
    topology.recovery_reserve.processes as u64
        | ((topology.recovery_reserve.channels as u64) << 16)
        | ((topology.recovery_reserve.handles as u64) << 32)
        | ((topology.recovery_reserve.per_owner_handles as u64) << 48)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_sdk::abi::supervisor as s;

    fn usage() -> Usage {
        Usage {
            processes: DECLARED.base_system.processes,
            channels: DECLARED.base_system.channels,
            handles: DECLARED.base_system.handles,
            per_owner_handles: DECLARED.base_system.per_owner_handles,
            file_clients: DECLARED.base_system.file_clients,
            child_slots: 0,
        }
    }

    #[test]
    fn declares_bootstrap_roles_and_recovery_reserve() {
        assert_eq!(DECLARED.kernel.processes, 16);
        assert_eq!(DECLARED.kernel.channels, 24);
        assert_eq!(DECLARED.kernel.handles, 64);
        assert_eq!(DECLARED.kernel.handles_per_owner, 24);
        assert_eq!(DECLARED.file_client_slots, 4);
        assert_eq!(DECLARED.reserved_file_clients, 2);
        assert_eq!(DECLARED.child_slots, 6);
        assert_eq!(BASE_CHANNELS.len(), DECLARED.base_system.channels as usize);
        assert_eq!(DECLARED.recovery_reserve.processes, 1);
        assert_eq!(DECLARED.recovery_reserve.channels, 3);
        assert_eq!(DECLARED.recovery_reserve.handles, 6);
        assert_eq!(DECLARED.tasks_if_launched, DECLARED.file_access_child);
    }

    #[test]
    fn all_roles_have_the_declared_file_access_class() {
        for role in [s::SPIN, s::FAULT, s::FINISH] {
            assert_eq!(child_role(role), Some(ChildRole::ControlOnly));
        }
        for role in [
            s::READ,
            s::PROBE,
            s::WATCH,
            s::LOST_REPLY,
            s::LOST_OPERATION,
            s::LOST_ADMISSION,
            s::ADMISSION_SESSION,
            s::PRIVATE_ADMISSION_SESSION,
            s::SESSION,
            s::HELPER,
            s::TASKS,
            s::TASKS_OWNER,
        ] {
            assert_eq!(child_role(role), Some(ChildRole::FileAccess));
            assert_eq!(
                cost(&DECLARED, Admission::Child(ChildRole::FileAccess)).file_clients,
                1
            );
        }
        assert_eq!(child_role(u64::MAX), None);
    }

    #[test]
    fn six_children_exceed_the_old_bootstrap_without_using_recovery_headroom() {
        let mut current = usage();
        for _ in 0..2 {
            admit(&DECLARED, current, Admission::Child(ChildRole::FileAccess)).unwrap();
            current.processes += 1;
            current.channels += 2;
            current.handles += 4;
            current.per_owner_handles += 2;
            current.file_clients += 1;
            current.child_slots += 1;
        }
        for _ in 0..4 {
            admit(&DECLARED, current, Admission::Child(ChildRole::ControlOnly)).unwrap();
            current.processes += 1;
            current.channels += 1;
            current.handles += 2;
            current.per_owner_handles += 1;
            current.child_slots += 1;
        }
        assert_eq!(current.processes, 9);
        assert_eq!(current.channels, 12);
        assert_eq!(current.file_clients, DECLARED.file_client_slots);
        assert_eq!(
            admit(&DECLARED, current, Admission::Child(ChildRole::ControlOnly)),
            Err(Budget::ChildSlots)
        );
    }

    #[test]
    fn each_budget_refuses_before_a_resource_is_allocated() {
        let cases = [
            (Budget::Processes, ChildRole::ControlOnly, 15, 0, 0, 0, 2, 0),
            (Budget::Channels, ChildRole::ControlOnly, 3, 21, 0, 0, 2, 0),
            (Budget::Handles, ChildRole::ControlOnly, 3, 4, 58, 0, 2, 0),
            (
                Budget::PerOwnerHandles,
                ChildRole::ControlOnly,
                3,
                4,
                8,
                18,
                2,
                0,
            ),
            (Budget::FileClients, ChildRole::FileAccess, 3, 4, 8, 3, 4, 0),
            (Budget::ChildSlots, ChildRole::ControlOnly, 3, 4, 8, 3, 2, 6),
        ];
        for (
            expected,
            role,
            processes,
            channels,
            handles,
            per_owner_handles,
            file_clients,
            child_slots,
        ) in cases
        {
            let current = Usage {
                processes,
                channels,
                handles,
                per_owner_handles,
                file_clients,
                child_slots,
            };
            assert_eq!(
                admit(&DECLARED, current, Admission::Child(role)),
                Err(expected),
                "{expected:?}"
            );
        }
    }

    #[test]
    fn staged_operations_leave_reserve_and_adoption_can_spend_it() {
        assert!(admit(&DECLARED, usage(), Admission::Stage).is_ok());
        assert!(admit(&DECLARED, usage(), Admission::StartStaged).is_ok());
        let saturated_app_budget = Usage {
            processes: 15,
            channels: 21,
            handles: 58,
            per_owner_handles: 18,
            file_clients: 4,
            child_slots: 6,
        };
        assert!(admit(&DECLARED, saturated_app_budget, Admission::AdoptFiles).is_ok());
        assert_eq!(
            admit(
                &DECLARED,
                saturated_app_budget,
                Admission::Child(ChildRole::ControlOnly)
            ),
            Err(Budget::Processes)
        );
    }

    #[test]
    fn limits_words_keep_live_and_declared_values_separate() {
        assert_eq!(unpack_used_limit(pack_used_limit(7, 16)), (7, 16));
        assert_eq!(
            pack_reserve(&DECLARED),
            6u64 << 48 | 6u64 << 32 | 3u64 << 16 | 1
        );
    }
}
