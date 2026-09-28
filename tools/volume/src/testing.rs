// SPDX-License-Identifier: Apache-2.0
//! Test-only scratch directories for image files, removed on drop, and a
//! disposable v7 image that already carries retained history.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::Poll;

use rustic_fs::format7::RecordState;
use rustic_fs::{
    Disk, Error, Kind, PollDisk, PollDisk7, Publication7Phase, Volume7, WriteIdentity7,
};

use crate::command::{V7_IMAGE_SECTORS, parse_lineage};
use crate::disk::FileDisk;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustic-volume-v7-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The workspace directory every history fixture creates under `/workspaces`.
pub(crate) const HISTORY_NAME: &str = "history";
/// Its node id: the first identity a fresh volume assigns.
pub(crate) const HISTORY: u32 = 5;
/// Retry subject of the fixture's own records, distinct from the host subject
/// `add7` uses, so the fixture never holds a key `add7` could pick.
const HISTORY_SUBJECT: u64 = 2;

/// Which retained history a fixture image carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum History {
    /// `direct.bin` (6) and `owner.bin` (7), each with one retained terminal
    /// direct commit.
    Receipts,
    /// The same two files plus one unresolved admission on `direct.bin`.
    Admission,
}

/// Exclusively create a disposable v7 image at `dir/name` holding `history`
/// under `/workspaces/history`, written through the v7 owner only.
pub(crate) fn history7(dir: &TempDir, name: &str, lineage: &str, history: History) -> PathBuf {
    let image = dir.path().join(name);
    let mut disk = FileDisk::create_new(&image, V7_IMAGE_SECTORS).unwrap();
    let mut volume = Box::new(Volume7::EMPTY);
    volume
        .provision_into(&mut disk, parse_lineage(lineage).unwrap())
        .unwrap();
    let workspace = volume
        .create(&mut disk, 4, HISTORY_NAME.as_bytes(), Kind::Directory)
        .unwrap();
    assert_eq!(workspace.id, HISTORY);
    let epoch = volume.header().unwrap().epoch;
    let mut direct = None;
    for (name, seed, key) in [(&b"direct.bin"[..], 1u8, 0x51u64), (b"owner.bin", 2, 0x52)] {
        let node = volume.create(&mut disk, HISTORY, name, Kind::File).unwrap();
        let bytes: Vec<u8> = (0..1500u32).map(|index| seed ^ index as u8).collect();
        let record = volume
            .replace_tracked(
                &mut disk,
                identity(node.id, node.version, epoch, key),
                node.version,
                &bytes,
            )
            .unwrap();
        assert_eq!(record.state, RecordState::DirectCommitted);
        direct.get_or_insert((node.id, record.committed));
    }
    if history == History::Admission {
        let (id, version) = direct.unwrap();
        let bytes = [0x5a; 700];
        let mut polled = Synchronous(&mut disk);
        let mut publication = volume
            .prepare_admission(
                &mut polled,
                identity(id, version, epoch, 0x53),
                version,
                &bytes,
            )
            .unwrap();
        let record = loop {
            match publication.poll_advance() {
                Poll::Ready(Ok(Publication7Phase::Committed)) => {
                    break publication.result().unwrap();
                }
                Poll::Ready(Ok(
                    Publication7Phase::Retrying
                    | Publication7Phase::Preparing
                    | Publication7Phase::ReadyToPublish
                    | Publication7Phase::Settling,
                ))
                | Poll::Pending => (),
                Poll::Ready(other) => panic!("admission did not settle: {other:?}"),
            }
        };
        assert_eq!(record.state, RecordState::Admitted);
    }
    image
}

fn identity(object: u32, instance: u64, retry_epoch: u64, retry_key: u64) -> WriteIdentity7 {
    WriteIdentity7 {
        subject: HISTORY_SUBJECT,
        workspace: HISTORY,
        object,
        instance,
        retry_epoch,
        retry_key,
    }
}

/// Every command settles inside the call, so the admission publication above
/// never observes `Pending` from the image.
struct Synchronous<'a>(&'a mut FileDisk);

impl PollDisk for Synchronous<'_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.write(sector, bytes))
    }

    fn poll_flush(&mut self) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.flush())
    }
}

impl PollDisk7 for Synchronous<'_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), Error>> {
        Poll::Ready(self.0.read(sector, bytes))
    }
}
