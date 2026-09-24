// SPDX-License-Identifier: Apache-2.0
//! Disposable v5 sources with scoped retained history, built only to exercise
//! the deliberate v5 -> v7 migration (`migrate7`) and its guest evidence.
//!
//! A v5 volume retains at most two records ([`rustic_fs::RETAINED`]), so the
//! history the migration must carry is split over three named sets. Every
//! record except the one deliberately left to subject 1 belongs to subject 2,
//! the V7 shell's retry scope, so the migrated records are visible to it. Real
//! v5 terminal volumes record their operations under subject 1 (the shell's
//! owner client); this seed does not claim that such records become visible to
//! the V7 shell, and no subject remapping is performed.
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use rustic_fs::{
    AdmissionStatus, Error, Kind, Node, PreventionReason, Publication, Replacement, Retry, Volume,
    format7,
};

use crate::command::{IMAGE_SECTORS, hex, parse_lineage, resource_text, workspace_text};
use crate::disk::FileDisk;

/// Retry scope of the V7 shell (`apps/supervisor/src/shell_binding.rs`).
pub(crate) const SHELL_SUBJECT: u64 = 2;
/// Retry scope of the v5 terminal's shell owner client and supervisor.
pub(crate) const OWNER_SUBJECT: u64 = 1;
/// The workspace directory under `/workspaces` that holds every seeded file.
const WORKSPACE_NAME: &[u8] = b"migrated";
/// Every seeded retry key lives in the first v5 retry epoch.
const EPOCH: u64 = 1;

/// Which two-record history a source carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistorySet {
    /// A current direct commit of subject 2 and one of subject 1.
    Receipts,
    /// An unresolved admission and a `Requested` cancellation, both subject 2.
    Admissions,
    /// An executed admission of subject 2.
    Completed,
}

impl HistorySet {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "receipts" => Ok(Self::Receipts),
            "admissions" => Ok(Self::Admissions),
            "completed" => Ok(Self::Completed),
            _ => Err("history set must be receipts, admissions or completed".to_owned()),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Receipts => "receipts",
            Self::Admissions => "admissions",
            Self::Completed => "completed",
        }
    }
}

/// The V7 shell's `replace-pattern-v7` content: byte `i` of seed `s` is
/// `s*31 + 7*i + i/509` modulo 256 (`apps/shell/src/commands/operations.rs`).
pub(crate) fn pattern(seed: u8, size: usize) -> Vec<u8> {
    (0..size as u32)
        .map(|index| {
            seed.wrapping_mul(31)
                .wrapping_add(index.wrapping_mul(7) as u8)
                .wrapping_add((index / 509) as u8)
        })
        .collect()
}

/// One seeded operation, described for the harness that replays it.
struct Seeded {
    state: &'static str,
    cause: Option<&'static str>,
    subject: u64,
    file: Node,
    key: u64,
    seed: u8,
    size: usize,
    previous: u64,
    committed: u64,
    admission: u64,
    terminal: u64,
}

/// Exclusively create a v5 image of the legacy tool size that carries `set`.
/// The provisioning envelope is written first, as the v5 terminal host does,
/// so the fresh volume adopts `lineage` as its recovery identity. An image this
/// call created is removed again if seeding fails.
pub(crate) fn seed5_history(image: &Path, lineage: &str, set: &str) -> Result<String, String> {
    let lineage = parse_lineage(lineage)?;
    let set = HistorySet::parse(set)?;
    drop(FileDisk::create_new(image, IMAGE_SECTORS)?);
    let result = seed(image, lineage, set);
    if result.is_err() {
        let _ = std::fs::remove_file(image);
    }
    result
}

fn seed(image: &Path, lineage: [u8; 16], set: HistorySet) -> Result<String, String> {
    write_envelope(image, lineage)?;
    let mut disk = FileDisk::open(image)?;

    let mut volume = Volume::initialize(&mut disk).map_err(refused("initialize"))?;
    volume
        .enable_operations(&mut disk)
        .map_err(refused("operations"))?;
    volume
        .enable_admissions(&mut disk)
        .map_err(refused("admissions"))?;
    volume
        .enable_prevention_reasons(&mut disk)
        .map_err(refused("prevention reasons"))?;
    let workspace = volume
        .create(&mut disk, 4, WORKSPACE_NAME, Kind::Directory)
        .map_err(refused("workspace"))?;

    let mut history = History {
        disk: &mut disk,
        volume: &mut volume,
        lineage,
        workspace: workspace.id,
        instance: 0,
    };
    let records = match set {
        HistorySet::Receipts => vec![
            history.direct(SHELL_SUBJECT, b"direct.bin", 0x500, 5, 700)?,
            history.direct(OWNER_SUBJECT, b"owner.bin", 0x501, 6, 300)?,
        ],
        HistorySet::Admissions => vec![
            history.admit(b"admitted.bin", 0x510, 7, 900)?,
            history.cancelled(b"cancelled.bin", 0x511, 8, 400)?,
        ],
        HistorySet::Completed => vec![history.executed(b"completed.bin", 0x520, 9, 1000)?],
    };
    let instance = history.instance;

    let (recovered_lineage, epoch) = volume.recovery_info().map_err(refused("recovery"))?;
    if recovered_lineage != lineage || epoch != EPOCH {
        return Err("seeded volume did not adopt the envelope lineage and first epoch".to_owned());
    }
    let mut record_texts = Vec::with_capacity(records.len());
    for record in &records {
        record_texts.push(record_json(record, lineage, workspace.id)?);
    }
    Ok(format!(
        "{{\"set\":\"{}\",\"lineage\":\"{}\",\"sequence\":{},\"epoch\":{EPOCH},\
         \"instance\":{instance},\"workspace\":{{\"id\":{},\"text\":\"{}\"}},\"records\":[{}]}}",
        set.name(),
        hex(&lineage),
        volume.sequence(),
        workspace.id,
        workspace_text(lineage, workspace.id)?,
        record_texts.join(","),
    ))
}

fn record_json(record: &Seeded, lineage: [u8; 16], workspace: u32) -> Result<String, String> {
    let cause = record
        .cause
        .map_or_else(|| "null".to_owned(), |cause| format!("\"{cause}\""));
    let name = String::from_utf8_lossy(record.file.name()).into_owned();
    Ok(format!(
        "{{\"state\":\"{}\",\"cause\":{cause},\"subject\":{},\"object\":{},\"name\":\"{name}\",\
         \"resource\":\"{}\",\"key\":{},\"seed\":{},\"size\":{},\"sha256\":\"{}\",\
         \"previous\":{},\"committed\":{},\"admission\":{},\"terminal\":{}}}",
        record.state,
        record.subject,
        record.file.id,
        resource_text(lineage, workspace, record.file.id)?,
        record.key,
        record.seed,
        record.size,
        crate::migrate7::sha256_hex(&pattern(record.seed, record.size)),
        record.previous,
        record.committed,
        record.admission,
        record.terminal,
    ))
}

/// Write the v5 provisioning envelope (`RUSTVOL1`, lineage, CRC32) to sector 1,
/// exactly as `tools/terminal_support/provision.py` does for a terminal disk.
fn write_envelope(image: &Path, lineage: [u8; 16]) -> Result<(), String> {
    let mut sector = [0u8; 512];
    sector[..8].copy_from_slice(b"RUSTVOL1");
    sector[8..24].copy_from_slice(&lineage);
    let checksum = format7::aggregate(&sector);
    sector[24..28].copy_from_slice(&checksum.to_le_bytes());
    let mut file = OpenOptions::new()
        .write(true)
        .open(image)
        .map_err(|error| format!("cannot open {}: {error}", image.display()))?;
    file.seek(SeekFrom::Start(512))
        .and_then(|_| file.write_all(&sector))
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot write the envelope: {error}"))
}

/// The seeding session: one v5 service incarnation writing into one workspace.
struct History<'a> {
    disk: &'a mut FileDisk,
    volume: &'a mut Volume,
    lineage: [u8; 16],
    workspace: u32,
    /// The incarnation's first committed sequence, fixed by its first operation.
    instance: u64,
}

impl History<'_> {
    fn file(&mut self, name: &[u8]) -> Result<Node, String> {
        self.volume
            .create(self.disk, self.workspace, name, Kind::File)
            .map_err(|error| format!("seed5-history file refused: {error:?}"))
    }

    fn instance(&mut self) -> u64 {
        if self.instance == 0 {
            self.instance = self.volume.sequence() + 1;
        }
        self.instance
    }

    fn request(&self, file: &Node, key: u64) -> Replacement {
        Replacement {
            workspace: self.workspace,
            retry: Retry {
                lineage: self.lineage,
                epoch: EPOCH,
                key,
            },
            id: file.id,
            version: file.version,
        }
    }

    fn direct(
        &mut self,
        subject: u64,
        name: &[u8],
        key: u64,
        seed: u8,
        size: usize,
    ) -> Result<Seeded, String> {
        let file = self.file(name)?;
        let instance = self.instance();
        let request = self.request(&file, key);
        let receipt = self
            .volume
            .replace_scoped(self.disk, subject, instance, request, &pattern(seed, size))
            .map_err(|error| format!("seed5-history direct commit refused: {error:?}"))?;
        Ok(Seeded {
            state: "direct_committed",
            cause: None,
            subject,
            file,
            key,
            seed,
            size,
            previous: receipt.previous,
            committed: receipt.committed,
            admission: 0,
            terminal: receipt.committed,
        })
    }

    fn admission(
        &mut self,
        name: &[u8],
        key: u64,
        seed: u8,
        size: usize,
    ) -> Result<(Node, AdmissionStatus), String> {
        let file = self.file(name)?;
        let instance = self.instance();
        let request = self.request(&file, key);
        let status = self
            .volume
            .admit_replace(
                self.disk,
                SHELL_SUBJECT,
                instance,
                request,
                &pattern(seed, size),
            )
            .map_err(|error| format!("seed5-history admission refused: {error:?}"))?;
        Ok((file, status))
    }

    fn admit(&mut self, name: &[u8], key: u64, seed: u8, size: usize) -> Result<Seeded, String> {
        let (file, status) = self.admission(name, key, seed, size)?;
        Ok(Seeded {
            state: "admitted",
            cause: None,
            subject: SHELL_SUBJECT,
            previous: file.version,
            file,
            key,
            seed,
            size,
            committed: 0,
            admission: status.id.number,
            terminal: 0,
        })
    }

    fn cancelled(
        &mut self,
        name: &[u8],
        key: u64,
        seed: u8,
        size: usize,
    ) -> Result<Seeded, String> {
        let (file, status) = self.admission(name, key, seed, size)?;
        let publication = self
            .volume
            .prepare_prevention(
                self.disk,
                SHELL_SUBJECT,
                status.id,
                PreventionReason::Requested,
            )
            .map_err(|error| format!("seed5-history cancellation refused: {error:?}"))?;
        let cancelled = finish(publication, "cancellation")?;
        Ok(Seeded {
            state: "cancelled",
            cause: Some("requested"),
            subject: SHELL_SUBJECT,
            previous: file.version,
            file,
            key,
            seed,
            size,
            committed: 0,
            admission: status.id.number,
            terminal: cancelled.terminal,
        })
    }

    fn executed(&mut self, name: &[u8], key: u64, seed: u8, size: usize) -> Result<Seeded, String> {
        let (file, status) = self.admission(name, key, seed, size)?;
        let publication = self
            .volume
            .prepare_admitted(self.disk, SHELL_SUBJECT, status.id)
            .map_err(|error| format!("seed5-history execution refused: {error:?}"))?;
        let receipt = finish(publication, "execution")?;
        Ok(Seeded {
            state: "admitted_committed",
            cause: None,
            subject: SHELL_SUBJECT,
            previous: receipt.previous,
            file,
            key,
            seed,
            size,
            committed: receipt.committed,
            admission: status.id.number,
            terminal: receipt.committed,
        })
    }
}

fn refused(what: &'static str) -> impl Fn(Error) -> String {
    move |error| format!("seed5-history {what} refused: {error:?}")
}

/// Drive a v5 publication to its durable result.
fn finish<T: Copy>(mut publication: Publication<'_, FileDisk, T>, what: &str) -> Result<T, String> {
    while publication.result().is_none() {
        publication
            .advance()
            .map_err(|error| format!("seed5-history {what} publication failed: {error:?}"))?;
    }
    publication
        .result()
        .ok_or_else(|| format!("seed5-history {what} has no result"))
}
