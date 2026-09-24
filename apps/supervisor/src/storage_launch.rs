// SPDX-License-Identifier: Apache-2.0
//! Supervisor policy for starting the one storage-sourced staged child, without transport.
//!
//! Staging admitted the image against [`crate::image_pair::STORAGE_FEATURES`],
//! which is admission only: the kernel checked what the manifest asks for, not
//! what it will be given. Starting the child is a second, separate supervisor
//! decision owned here: which manifest identity may run from storage at all,
//! which roles it may be asked to perform, and which authority that role is
//! issued. The supervisor binary owns the kernel calls and the channel.
//!
//! There is exactly one topology: control-only. The child receives one private
//! control channel to the supervisor, which carries the role message and the
//! child's report. It receives no file endpoint, no file-service peer, no block
//! grant and no console, so the admitted roles are exactly the utility roles that
//! need none of them. The manifest identity is the name the manifest declares;
//! its SHA-256 binding pins the ELF bytes but does not authenticate a publisher.
use rustic_sdk::abi::{
    application::{self, Manifest},
    runtime::Error as RuntimeError,
    supervisor::{self as s, launch},
};

/// The only manifest identity the supervisor starts from storage.
pub const IDENTITY: &str = "rustic.utility";

/// Owner status for a kernel refusal of the control channel or of the start.
pub fn kernel_refusal(error: RuntimeError) -> u64 {
    launch::KERNEL_ERROR_BASE + error as u64
}

/// What the supervisor retains from the staged manifest. It is captured from
/// the exact 128 bytes the kernel admitted, so it describes the running image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    admitted_identity: bool,
    requests: u64,
    version: [u16; 3],
}

impl Facts {
    /// Keep the facts of an admitted manifest; `None` for bytes that are not a
    /// valid manifest at all.
    pub fn parse(manifest: &[u8]) -> Option<Self> {
        let manifest = Manifest::parse(manifest).ok()?;
        Some(Self {
            admitted_identity: manifest.identity == IDENTITY,
            requests: manifest.requests,
            version: manifest.version,
        })
    }

    /// The application features the manifest requested.
    pub fn requests(&self) -> u64 {
        self.requests
    }

    /// The application version the manifest declared.
    pub fn version(&self) -> [u16; 3] {
        self.version
    }
}

/// The authority issued to the started child. Only [`plan`] constructs it, so
/// holding one means the policy accepted the identity, role and features.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Topology {
    role: u64,
}

impl Topology {
    /// Features implied by what is issued: the control channel is IPC.
    pub const FEATURES: u64 = application::IPC;

    pub fn role(&self) -> u64 {
        self.role
    }

    /// First message on the control channel, in the utility's role layout
    /// `[role, scope, other, generation, ..]`: no scope, second object or grant
    /// generation, because no file authority exists.
    pub fn role_message(&self) -> [u64; 8] {
        [self.role, 0, 0, 0, 0, 0, 0, 0]
    }

    /// Kernel start words `[data token, control token, file-service pid]` for the
    /// child's end of the control channel: the data token and the peer are zero.
    pub fn start_arguments(&self, control: u64) -> [u64; 3] {
        [0, control, 0]
    }
}

/// Decide whether the staged child described by `facts` may be started in
/// `role`, checking identity, then role, then features.
pub fn plan(facts: &Facts, role: u64) -> Result<Topology, u64> {
    if !facts.admitted_identity {
        return Err(launch::IDENTITY);
    }
    // The utility roles that use neither the file endpoint nor the file-service
    // peer. Every other role, known or not, needs authority this topology lacks.
    if !matches!(role, s::FINISH | s::FAULT | s::SPIN) {
        return Err(launch::ROLE);
    }
    if facts.requests & Topology::FEATURES != Topology::FEATURES {
        return Err(launch::FEATURES);
    }
    Ok(Topology { role })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema-2 manifest as `tools/application.py` encodes it.
    fn manifest(identity: &str, requests: u64) -> [u8; application::SIZE] {
        let mut bytes = [0; application::SIZE];
        bytes[..8].copy_from_slice(b"RUSTAPP\0");
        bytes[8..10].copy_from_slice(&application::VERSION.to_le_bytes());
        bytes[10..12].copy_from_slice(&(application::SIZE as u16).to_le_bytes());
        bytes[12..16].copy_from_slice(&(rustic_sdk::abi::process::VERSION as u32).to_le_bytes());
        bytes[16..18].copy_from_slice(&rustic_sdk::abi::ipc::VERSION.to_le_bytes());
        for (i, part) in [0u16, 1, 2].iter().enumerate() {
            bytes[18 + 2 * i..20 + 2 * i].copy_from_slice(&part.to_le_bytes());
        }
        bytes[24..32].copy_from_slice(&requests.to_le_bytes());
        bytes[32..32 + identity.len()].copy_from_slice(identity.as_bytes());
        bytes[64..75].copy_from_slice(b"utility.elf");
        bytes
    }

    fn facts(identity: &str, requests: u64) -> Facts {
        Facts::parse(&manifest(identity, requests)).unwrap()
    }

    #[test]
    fn the_utility_manifest_starts_control_only_roles() {
        let utility = facts(IDENTITY, application::IPC);
        assert_eq!(utility.version(), [0, 1, 2]);
        assert_eq!(utility.requests(), application::IPC);
        for role in [s::FINISH, s::FAULT, s::SPIN] {
            let topology = plan(&utility, role).unwrap();
            assert_eq!(topology.role(), role);
            assert_eq!(topology.role_message(), [role, 0, 0, 0, 0, 0, 0, 0]);
            // Only the control end is handed over: no data token, no peer.
            assert_eq!(topology.start_arguments(42), [0, 42, 0]);
        }
    }

    #[test]
    fn roles_that_need_files_or_are_unknown_are_refused() {
        let utility = facts(IDENTITY, application::IPC);
        for role in [
            0,
            s::READ,
            s::PROBE,
            s::WATCH,
            s::LOST_REPLY,
            s::LOST_OPERATION,
            s::LOST_ADMISSION,
            s::SESSION,
            s::HELPER,
            s::ADMISSION_SESSION,
            s::PRIVATE_ADMISSION_SESSION,
            s::TASKS,
            s::TASKS_OWNER,
            99,
            u64::MAX,
        ] {
            assert_eq!(plan(&utility, role), Err(launch::ROLE), "role {role}");
        }
    }

    #[test]
    fn other_identities_are_refused_before_the_role_is_considered() {
        // The shipped file-server manifest is admitted for staging, never started.
        let file_server = facts("rustic.file-server", application::IPC | application::BLOCK);
        assert_eq!(plan(&file_server, s::FINISH), Err(launch::IDENTITY));
        assert_eq!(plan(&file_server, s::READ), Err(launch::IDENTITY));
        for identity in ["rustic.utilityx", "rustic.utilit", "utility"] {
            assert_eq!(
                plan(&facts(identity, application::IPC), s::FINISH),
                Err(launch::IDENTITY)
            );
        }
    }

    #[test]
    fn the_issued_channel_must_have_been_requested() {
        assert_eq!(plan(&facts(IDENTITY, 0), s::FINISH), Err(launch::FEATURES));
        assert_eq!(
            plan(&facts(IDENTITY, application::BLOCK), s::FINISH),
            Err(launch::FEATURES)
        );
        // Requesting more than is issued is admission's concern, not a refusal here.
        let broad = facts(IDENTITY, application::IPC | application::BLOCK);
        assert!(plan(&broad, s::FINISH).is_ok());
    }

    #[test]
    fn malformed_manifests_carry_no_facts() {
        let mut bytes = manifest(IDENTITY, application::IPC);
        bytes[0] = b'X';
        assert_eq!(Facts::parse(&bytes), None);
        assert_eq!(Facts::parse(&bytes[..64]), None);
    }

    #[test]
    fn kernel_refusals_share_the_staging_encoding() {
        assert_eq!(kernel_refusal(RuntimeError::Full), 64 + 6);
        assert_eq!(kernel_refusal(RuntimeError::Busy), 64 + 4);
        assert_eq!(
            launch::KERNEL_ERROR_BASE,
            s::stage::KERNEL_ERROR_BASE,
            "the shell renders both with one decoder"
        );
        // Owner statuses 5 and 6 mean a job and a superseded job; never reused.
        for status in [
            launch::IDENTITY,
            launch::ROLE,
            launch::FEATURES,
            launch::STARTED,
        ] {
            assert!(!(1..=6).contains(&status));
            assert!(status < launch::KERNEL_ERROR_BASE);
        }
    }
}
