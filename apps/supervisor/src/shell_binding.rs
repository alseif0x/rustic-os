// SPDX-License-Identifier: Apache-2.0
//! The shell's V7 file binding: which authority the supervisor grants it and
//! how an owner revocation replaces it, without transport.
//!
//! The supervisor binary owns the channels, the exchanges and the order of the
//! owner's [`REVOKE_SHELL_V7`](rustic_sdk::abi::supervisor::REVOKE_SHELL_V7)
//! job (revoke, then the mount's channel and grant phases); this module owns
//! the administrative words and the checks on their replies, so they are
//! exercised directly by host tests. The policy is fixed: the shell always
//! holds file-service client slot 0 with the tracked-write profile under
//! subject 2, the retry scope reserved for it.
use rustic_sdk::abi::files::{GRANT, REVOKE};

/// File-service client slot of the shell's binding.
pub const SLOT: u64 = 0;
/// Read, write and inspect: the V7 tracked-write profile.
pub const RIGHTS: u64 = 7;
/// Retry scope of the shell's tracked writes, distinct from the host
/// provisioner's subject 1.
pub const SUBJECT: u64 = 2;

/// Administrative words that install the shell's binding on `endpoint`, the
/// service-side end of a channel to `shell`, scoped to the workspaces root.
pub fn grant(shell: u64, endpoint: u64) -> [u64; 8] {
    [
        u64::from(GRANT),
        SLOT,
        shell,
        endpoint,
        0,
        RIGHTS,
        0,
        SUBJECT,
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

/// Whether the service confirmed the revocation. Only an all-zero reply does.
pub fn revoked(reply: [u64; 8]) -> bool {
    reply == [0; 8]
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
    fn the_shell_grant_is_slot_zero_tracked_write_subject_two_at_the_workspaces_root() {
        assert_eq!(grant(3, 17), [32, 0, 3, 17, 0, 7, 0, 2]);
        assert_eq!(revoke(), [33, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(rebound(5, 9, 12), [0, 5, 9, 12, 0, 0, 0, 0]);
    }

    #[test]
    fn only_a_clean_revocation_reply_confirms_it() {
        assert!(revoked([0; 8]));
        for reply in [
            [18, 0, 0, 0, 0, 0, 0, 0],
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
