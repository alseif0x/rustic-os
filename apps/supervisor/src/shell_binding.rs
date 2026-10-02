// SPDX-License-Identifier: Apache-2.0
//! The shell's file binding: which authority the supervisor grants it and how
//! an owner revocation replaces it, without transport.
//!
//! The supervisor binary owns the channels, the exchanges and the order of the
//! owner's [`REVOKE_SHELL_V7`](rustic_sdk::abi::supervisor::REVOKE_SHELL_V7)
//! job (revoke, then the mount's channel and grant phases); this module owns
//! the administrative words and the checks on their replies, so they are
//! exercised directly by host tests. The selected shell policy is independent
//! of the mounted file format.
use rustic_sdk::abi::files::{GRANT, REVOKE};

/// Authority selected for the shell independently of the mounted file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellPolicy {
    /// Whole-volume manual policy, sharing the host provisioner's retry subject.
    Manual,
    /// Workspace-only policy with a distinct shell retry subject.
    Workspace,
}

/// File-service client slot of the shell's binding.
pub const SLOT: u64 = 0;
/// Full shell file authority; the mounted format determines which operations exist.
pub const RIGHTS: u64 = 15;
/// Administrative words that install the shell's binding on `endpoint`, the
/// service-side end of a channel to `shell`, under the selected policy.
pub fn grant(shell: u64, endpoint: u64, policy: ShellPolicy) -> [u64; 8] {
    let (scope, subject) = match policy {
        ShellPolicy::Manual => (0, 1),
        ShellPolicy::Workspace => (4, 2),
    };
    [
        u64::from(GRANT),
        SLOT,
        shell,
        endpoint,
        scope,
        RIGHTS,
        0,
        subject,
    ]
}

/// Administrative words that revoke the shell's binding.
pub fn revoke() -> [u64; 8] {
    [u64::from(REVOKE), SLOT, 0, 0, 0, 0, 0, 0]
}

/// The generation of an accepted grant, or `None` for any other reply.
pub fn granted(reply: [u64; 8]) -> Option<u32> {
    if reply[0] != 0 || reply[2..].iter().any(|word| *word != 0) {
        return None;
    }
    u32::try_from(reply[1])
        .ok()
        .filter(|generation| *generation != 0)
}

/// Whether the service confirmed revocation with an unfenced settlement report.
pub fn revoked(reply: [u64; 8]) -> bool {
    reply[0] == 0
        && reply[1] & 1 != 0
        && reply[1] & !0x0f == 0
        && reply[2] <= 2
        && reply[3] == 0
        && reply[4] != 0
        && reply[5..].iter().all(|word| *word == 0)
}

/// The owner job's result: the same binding words a restart reports, which
/// the shell adopts as its new file binding.
pub fn rebound(files: u64, endpoint: u64, generation: u32) -> [u64; 8] {
    [0, files, endpoint, u64::from(generation), 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_shell_policy_covers_the_volume_under_subject_one() {
        assert_eq!(
            grant(3, 17, ShellPolicy::Manual),
            [32, 0, 3, 17, 0, 15, 0, 1]
        );
    }

    #[test]
    fn workspace_shell_policy_is_scoped_under_subject_two() {
        assert_eq!(
            grant(3, 17, ShellPolicy::Workspace),
            [32, 0, 3, 17, 4, 15, 0, 2]
        );
        assert_eq!(revoke(), [33, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(rebound(5, 9, 12), [0, 5, 9, 12, 0, 0, 0, 0]);
    }

    #[test]
    fn only_an_unfenced_settlement_report_confirms_revocation() {
        assert!(revoked([0, 1, 0, 0, 12, 0, 0, 0]));
        for reply in [
            [18, 0, 0, 0, 0, 0, 0, 0],
            [0; 8],
            [0, 0, 0, 0, 12, 0, 0, 0],
            [0, 1, 3, 0, 12, 0, 0, 0],
            [0, 1, 0, 2, 12, 0, 0, 0],
            [0, 1, 0, 1, 12, 0, 0, 0],
            [0, 1, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 1],
        ] {
            assert!(!revoked(reply));
        }
    }

    #[test]
    fn only_a_clean_nonzero_generation_is_an_accepted_grant() {
        assert_eq!(granted([0, 4, 0, 0, 0, 0, 0, 0]), Some(4));
        for reply in [
            [4, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0],
            [0, 4, 1, 0, 0, 0, 0, 0],
            [0, 1 << 32, 0, 0, 0, 0, 0, 0],
        ] {
            assert_eq!(granted(reply), None);
        }
    }
}
