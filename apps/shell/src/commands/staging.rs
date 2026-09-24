// SPDX-License-Identifier: Apache-2.0
//! Owner requests to stage one V7 ELF/manifest pair as a dormant child and to
//! start that child.
//!
//! The shell only names the pair; the supervisor reads it with its own read-only
//! authority and reports the outcome as an ordinary owner job. Starting names the
//! staged child and a role; the supervisor alone decides whether the staged
//! manifest and the role qualify and issues the control-only topology.
use super::*;
use rustic_sdk::abi::{
    files::{
        Error as FileError,
        reference::{Resource, Version, Workspace},
    },
    runtime::Error as RuntimeError,
    supervisor::{self as p, launch, stage},
};

pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    if argument(a, 0)? == "start-staged" {
        return start(s, a);
    }
    exact(a, 6)?;
    let workspace = argument(a, 1)?.parse::<Workspace>()?;
    let elf = argument(a, 2)?.parse::<Resource>()?;
    let manifest = argument(a, 3)?.parse::<Resource>()?;
    let elf_version = argument(a, 4)?.parse::<Version>()?;
    let manifest_version = argument(a, 5)?.parse::<Version>()?;
    if elf.workspace() != workspace || manifest.workspace() != workspace {
        return Err(Error::Usage);
    }
    let lineage = workspace.lineage();
    let r = s.request([
        p::STAGE_V7,
        u64::from_le_bytes(lineage[..8].try_into().map_err(|_| Error::Usage)?),
        u64::from_le_bytes(lineage[8..].try_into().map_err(|_| Error::Usage)?),
        workspace.root().into(),
        elf.object().into(),
        manifest.object().into(),
        elf_version.value(),
        manifest_version.value(),
    ])?;
    if r[0] != 5 {
        return Err(Error::Service(4));
    }
    output::format(format_args!(
        "stage requested job={} (dormant child only; query with job-status {})\r\n",
        r[1], r[1]
    ));
    Ok(())
}

/// Role names the shell can send. It forwards roles the supervisor refuses too,
/// so the storage launch policy is decided in one place.
fn role(name: &str) -> Result<u64, Error> {
    Ok(match name {
        "exit" => p::FINISH,
        "fault" => p::FAULT,
        "spin" => p::SPIN,
        "read" => p::READ,
        "session" => p::SESSION,
        _ => return Err(Error::Usage),
    })
}

fn start(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
    exact(a, 3)?;
    let pid = number(a, 1)?;
    let name = argument(a, 2)?;
    let r = s
        .request([p::START_STAGED, pid, role(name)?, 0, 0, 0, 0, 0])
        .map_err(|error| match error {
            Error::Service(code)
                if matches!(
                    code,
                    launch::IDENTITY | launch::ROLE | launch::FEATURES | launch::STARTED
                ) || code >= launch::KERNEL_ERROR_BASE =>
            {
                Error::StartRefused(code)
            }
            error => error,
        })?;
    if r[0] != 0 || r[1] != pid {
        return Err(Error::Service(4));
    }
    output::format(format_args!(
        "started staged pid={pid} role={name} topology=control-only\r\n"
    ));
    Ok(())
}

/// Readable class of a start refusal status.
pub(super) fn start_refusal(f: &mut core::fmt::Formatter<'_>, code: u64) -> core::fmt::Result {
    match code {
        launch::IDENTITY => f.write_str("manifest identity is not started from storage"),
        launch::ROLE => {
            f.write_str("role needs authority the control-only topology does not issue")
        }
        launch::FEATURES => f.write_str("manifest does not request the ipc feature"),
        launch::STARTED => f.write_str("already started"),
        code => kernel(f, code - launch::KERNEL_ERROR_BASE),
    }
}

fn kernel(f: &mut core::fmt::Formatter<'_>, index: u64) -> core::fmt::Result {
    match (index < 10).then(|| RuntimeError::decode(RuntimeError::Denied.code() - index)) {
        Some(Err(error)) => write!(f, "kernel {error:?}"),
        _ => f.write_str("kernel error"),
    }
}

/// Render one completed stage job. A refusal is returned as an error after its
/// status line so the command status reflects it.
pub(super) fn display(r: [u64; 8]) -> Result<(), Error> {
    if r[0] != 0 || r[7] != 0 {
        return Err(Error::Service(4));
    }
    if r[3] != 0 {
        output::format(format_args!(
            "job={} complete kind={} status={}\r\n",
            r[1], r[2], r[3]
        ));
        return Err(Error::StageRefused(r[3]));
    }
    let (Ok(elf), Ok(manifest)) = (Version::new(r[5]), Version::new(r[6])) else {
        return Err(Error::Service(4));
    };
    output::format(format_args!(
        "job={} complete kind={} status=0 staged pid={} elf_version={} manifest_version={} state=dormant\r\n",
        r[1], r[2], r[4], elf, manifest
    ));
    Ok(())
}

/// Readable class of a stage refusal status.
pub(super) fn refusal(f: &mut core::fmt::Formatter<'_>, code: u64) -> core::fmt::Result {
    match code {
        1 => f.write_str("invalid request"),
        2 => f.write_str("denied"),
        3 => f.write_str("busy"),
        6 => f.write_str("superseded by a service restart; transaction aborted"),
        stage::PAIR_MISMATCH => f.write_str("pair mismatch"),
        stage::IMAGE_SIZE => f.write_str("image size outside staging bounds"),
        code if (stage::FILE_ERROR_BASE + 1..stage::KERNEL_ERROR_BASE).contains(&code) => {
            match u8::try_from(code - stage::FILE_ERROR_BASE).map(FileError::parse) {
                Ok(Err(error)) => write!(f, "file {error:?}"),
                _ => f.write_str("file error"),
            }
        }
        code if (stage::KERNEL_ERROR_BASE..stage::KERNEL_ERROR_BASE + 10).contains(&code) => {
            kernel(f, code - stage::KERNEL_ERROR_BASE)
        }
        _ => f.write_str("service unavailable"),
    }
}
