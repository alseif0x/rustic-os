// SPDX-License-Identifier: Apache-2.0
//! Owner request to stage one V7 ELF/manifest pair as a dormant child.
//!
//! The shell only names the pair; the supervisor reads it with its own read-only
//! authority and reports the outcome as an ordinary owner job.
use super::*;
use rustic_sdk::abi::{
    files::{
        Error as FileError,
        reference::{Resource, Version, Workspace},
    },
    runtime::Error as RuntimeError,
    supervisor::{self as p, stage},
};

pub(super) fn execute(s: &mut Session, a: &Args<'_>) -> Result<(), Error> {
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
            match RuntimeError::decode(
                RuntimeError::Denied.code() - (code - stage::KERNEL_ERROR_BASE),
            ) {
                Err(error) => write!(f, "kernel {error:?}"),
                Ok(_) => f.write_str("kernel error"),
            }
        }
        _ => f.write_str("service unavailable"),
    }
}
