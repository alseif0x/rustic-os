// SPDX-License-Identifier: Apache-2.0
//! Owner-pinned pairing of one storage-sourced ELF with its manifest, without transport.
//!
//! The supervisor binary owns the file client, the reads and the kernel staging
//! calls; this module owns only what makes the two reads one version pair: which
//! ranges are requested, in which order, and which observations are accepted.
//! Every range must carry the owner-supplied version, the owner's workspace and
//! the retry epoch first observed with the manifest; the ELF ranges must be
//! contiguous, of constant size and end exactly at EOF.
//!
//! The pinned SHA-256 of each range is transport and pair integrity. It does not
//! authenticate a publisher, and nothing here ties the manifest's executable name
//! to the storage node that supplied the bytes.
use rustic_sdk::abi::{
    application,
    files::{
        Error as FileError,
        read::{Info, MAX_RANGE, Request},
        reference::{Epoch, Resource, Version, Workspace},
    },
    runtime::{self, Error as RuntimeError},
    supervisor::{STAGE_V7, stage},
};

/// Supervisor admission policy for images staged from storage: the requested
/// application features that may be admitted. It is the minimum set that admits
/// the shipped `file-server` artifact (IPC and block requests). Admission is not
/// a grant: the staged child receives no endpoint, block, console or control
/// authority from this request.
pub const STORAGE_FEATURES: u64 = application::IPC | application::BLOCK;

/// Largest range requested from the service; the transfer follows the service's
/// own bound instead of inventing another one.
const RANGE: u16 = MAX_RANGE as u16;

/// Owner status for a file-service refusal.
pub fn file_refusal(error: FileError) -> u64 {
    stage::FILE_ERROR_BASE + error as u64
}

/// Owner status for a kernel staging refusal.
pub fn kernel_refusal(error: RuntimeError) -> u64 {
    stage::KERNEL_ERROR_BASE + error as u64
}

/// The pair exactly as the owner named it. No field confers authority; the
/// supervisor's own read grant decides what the service will answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    workspace: Workspace,
    elf: Resource,
    manifest: Resource,
    elf_version: Version,
    manifest_version: Version,
}

impl Pin {
    /// Decode a complete [`STAGE_V7`] request. Every malformed word is refused
    /// with the owner's generic invalid status `1`.
    pub fn decode(words: [u64; 8]) -> Result<Self, u64> {
        if words[0] != STAGE_V7 {
            return Err(1);
        }
        let mut lineage = [0; 16];
        lineage[..8].copy_from_slice(&words[1].to_le_bytes());
        lineage[8..].copy_from_slice(&words[2].to_le_bytes());
        let object = |word: u64| u32::try_from(word).map_err(|_| 1u64);
        let workspace = Workspace::new(lineage, object(words[3])?).map_err(|_| 1u64)?;
        let elf = Resource::new(workspace, object(words[4])?).map_err(|_| 1u64)?;
        let manifest = Resource::new(workspace, object(words[5])?).map_err(|_| 1u64)?;
        if elf == manifest
            || elf.object() == workspace.root()
            || manifest.object() == workspace.root()
        {
            return Err(1);
        }
        Ok(Self {
            workspace,
            elf,
            manifest,
            elf_version: Version::new(words[6]).map_err(|_| 1u64)?,
            manifest_version: Version::new(words[7]).map_err(|_| 1u64)?,
        })
    }

    pub fn elf_version(&self) -> Version {
        self.elf_version
    }

    pub fn manifest_version(&self) -> Version {
        self.manifest_version
    }

    /// The single manifest range, pinned to the owner's manifest version. It asks
    /// for a whole service range so a larger file is observed and refused.
    pub fn manifest_request(&self) -> Request {
        Request {
            workspace: self.workspace,
            resource: self.manifest,
            expected_version: Some(self.manifest_version),
            offset: 0,
            length: RANGE,
        }
    }

    /// Accept the verified manifest range and return the retry epoch that every
    /// ELF range must repeat.
    pub fn accept_manifest(&self, info: &Info) -> Result<Epoch, u64> {
        if !self.observed(info, self.manifest, self.manifest_version)
            || info.offset != 0
            || info.size != application::SIZE as u64
            || info.length != application::SIZE
            || !info.eof
        {
            return Err(stage::PAIR_MISMATCH);
        }
        Ok(info.retry_epoch)
    }

    fn observed(&self, info: &Info, resource: Resource, version: Version) -> bool {
        info.references.workspace == self.workspace
            && info.references.resource == resource
            && info.version == version
    }
}

/// One accepted ELF range and what the caller must do with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub offset: u64,
    pub length: usize,
    /// Present on the first range only: the image length to declare when the
    /// kernel transaction is opened, before the range is copied into it.
    pub begin: Option<u64>,
    /// The range ended exactly at EOF; the transaction may be committed.
    pub last: bool,
}

/// Contiguous progress over the pinned ELF. The size is learned from the first
/// range and must not change afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transfer {
    pin: Pin,
    epoch: Epoch,
    size: Option<u64>,
    next: u64,
    ranges: u32,
}

impl Transfer {
    pub fn new(pin: Pin, epoch: Epoch) -> Self {
        Self {
            pin,
            epoch,
            size: None,
            next: 0,
            ranges: 0,
        }
    }

    /// The next range to read, or `None` once the whole image was accepted.
    pub fn request(&self) -> Option<Request> {
        if self.complete() {
            return None;
        }
        Some(Request {
            workspace: self.pin.workspace,
            resource: self.pin.elf,
            expected_version: Some(self.pin.elf_version),
            offset: self.next,
            length: RANGE,
        })
    }

    pub fn complete(&self) -> bool {
        self.size.is_some_and(|size| self.next == size)
    }

    /// Image length once known.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// Ranges accepted so far.
    pub fn ranges(&self) -> u32 {
        self.ranges
    }

    /// Accept the verified observation of the range last returned by
    /// [`Self::request`]. A refusal leaves the transfer unchanged; the caller
    /// must abandon the whole pair rather than retry a range.
    pub fn accept(&mut self, info: &Info) -> Result<Accepted, u64> {
        if self.complete()
            || !self.pin.observed(info, self.pin.elf, self.pin.elf_version)
            || info.retry_epoch != self.epoch
            || info.offset != self.next
        {
            return Err(stage::PAIR_MISMATCH);
        }
        let begin = match self.size {
            Some(size) if size != info.size => return Err(stage::PAIR_MISMATCH),
            Some(_) => None,
            None => {
                let bounds =
                    runtime::MIN_STAGED_IMAGE_BYTES as u64..=runtime::MAX_STAGED_IMAGE_BYTES as u64;
                if !bounds.contains(&info.size) {
                    return Err(stage::IMAGE_SIZE);
                }
                Some(info.size)
            }
        };
        let remaining = info.size - self.next;
        let expected = remaining.min(u64::from(RANGE));
        let end = self.next + expected;
        if expected == 0 || info.length as u64 != expected || info.eof != (end == info.size) {
            return Err(stage::PAIR_MISMATCH);
        }
        let accepted = Accepted {
            offset: self.next,
            length: info.length,
            begin,
            last: info.eof,
        };
        self.size = Some(info.size);
        self.next = end;
        self.ranges += 1;
        Ok(accepted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_sdk::abi::files::reference::References;

    const LINEAGE: [u8; 16] = [7; 16];
    const ROOT: u32 = 4;
    const ELF: u32 = 9;
    const MANIFEST: u32 = 10;
    const ELF_VERSION: u64 = 21;
    const MANIFEST_VERSION: u64 = 22;
    const EPOCH: u64 = 3;

    fn words() -> [u64; 8] {
        [
            STAGE_V7,
            u64::from_le_bytes(LINEAGE[..8].try_into().unwrap()),
            u64::from_le_bytes(LINEAGE[8..].try_into().unwrap()),
            ROOT.into(),
            ELF.into(),
            MANIFEST.into(),
            ELF_VERSION,
            MANIFEST_VERSION,
        ]
    }

    fn pin() -> Pin {
        Pin::decode(words()).unwrap()
    }

    fn info(object: u32, version: u64, size: u64, offset: u64, length: usize) -> Info {
        Info {
            references: References::new(LINEAGE, ROOT, object).unwrap(),
            version: Version::new(version).unwrap(),
            size,
            offset,
            length,
            range_sha256: [0; 32],
            eof: offset + length as u64 == size,
            retry_epoch: Epoch::new(EPOCH).unwrap(),
        }
    }

    fn manifest() -> Info {
        info(MANIFEST, MANIFEST_VERSION, 128, 0, 128)
    }

    fn transfer() -> Transfer {
        Transfer::new(pin(), pin().accept_manifest(&manifest()).unwrap())
    }

    fn elf(size: u64, offset: u64) -> Info {
        let length = (size - offset).min(1024) as usize;
        info(ELF, ELF_VERSION, size, offset, length)
    }

    #[test]
    fn request_words_decode_into_one_pinned_pair() {
        let pin = pin();
        let manifest = pin.manifest_request();
        assert_eq!(manifest.resource.object(), MANIFEST);
        assert_eq!(manifest.workspace.lineage(), LINEAGE);
        assert_eq!(manifest.workspace.root(), ROOT);
        assert_eq!(manifest.expected_version.unwrap().value(), MANIFEST_VERSION);
        let first = Transfer::new(pin, Epoch::new(EPOCH).unwrap())
            .request()
            .unwrap();
        assert_eq!(first.resource.object(), ELF);
        assert_eq!(first.expected_version.unwrap().value(), ELF_VERSION);
        assert_eq!((first.offset, first.length), (0, 1024));
    }

    #[test]
    fn malformed_requests_are_invalid() {
        let mut cases = [words(); 10];
        cases[0][0] = STAGE_V7 + 1;
        cases[1][1] = 0;
        cases[1][2] = 0;
        cases[2][3] = 0;
        cases[3][4] = 0;
        cases[4][5] = u64::from(u32::MAX) + 1;
        cases[5][5] = ELF.into();
        cases[6][4] = ROOT.into();
        cases[7][6] = 0;
        cases[8][7] = 0;
        cases[9][5] = ROOT.into();
        for case in cases {
            assert_eq!(Pin::decode(case), Err(1), "{case:?}");
        }
    }

    #[test]
    fn manifest_must_be_one_exact_pinned_range() {
        let pin = pin();
        assert_eq!(pin.accept_manifest(&manifest()).unwrap().value(), EPOCH);
        let refusals = [
            info(ELF, MANIFEST_VERSION, 128, 0, 128),
            info(MANIFEST, MANIFEST_VERSION + 1, 128, 0, 128),
            info(MANIFEST, MANIFEST_VERSION, 129, 0, 129),
            info(MANIFEST, MANIFEST_VERSION, 127, 0, 127),
            info(MANIFEST, MANIFEST_VERSION, 1024, 0, 128),
            Info {
                references: References::new([8; 16], ROOT, MANIFEST).unwrap(),
                ..manifest()
            },
            Info {
                references: References::new(LINEAGE, ROOT + 1, MANIFEST).unwrap(),
                ..manifest()
            },
            Info {
                eof: false,
                ..manifest()
            },
        ];
        for refused in refusals {
            assert_eq!(
                pin.accept_manifest(&refused),
                Err(stage::PAIR_MISMATCH),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn contiguous_ranges_open_once_and_end_at_eof() {
        let size = 2 * 1024 + 100;
        let mut t = transfer();
        let first = t.accept(&elf(size, 0)).unwrap();
        assert_eq!(first.begin, Some(size));
        assert!(!first.last);
        let second = t.accept(&elf(size, 1024)).unwrap();
        assert_eq!(
            (second.offset, second.length, second.begin),
            (1024, 1024, None)
        );
        assert_eq!(t.request().unwrap().offset, 2048);
        let last = t.accept(&elf(size, 2048)).unwrap();
        assert_eq!((last.length, last.last), (100, true));
        assert!(t.complete());
        assert_eq!(t.request(), None);
        assert_eq!(t.ranges(), 3);
        assert_eq!(t.accept(&elf(size, 2048)), Err(stage::PAIR_MISMATCH));
    }

    #[test]
    fn a_single_range_image_is_opened_and_finished_by_one_range() {
        let mut t = transfer();
        let only = t.accept(&elf(64, 0)).unwrap();
        assert_eq!((only.begin, only.last, only.length), (Some(64), true, 64));
        assert!(t.complete());
    }

    #[test]
    fn inconsistent_ranges_are_refused_without_progress() {
        let size = 3000;
        let mut started = transfer();
        started.accept(&elf(size, 0)).unwrap();
        let snapshot = started;
        let refusals = [
            // Another version, resource, workspace or retry epoch than the pin.
            info(ELF, ELF_VERSION + 1, size, 1024, 1024),
            info(MANIFEST, ELF_VERSION, size, 1024, 1024),
            Info {
                references: References::new([9; 16], ROOT, ELF).unwrap(),
                ..elf(size, 1024)
            },
            Info {
                retry_epoch: Epoch::new(EPOCH + 1).unwrap(),
                ..elf(size, 1024)
            },
            // Out of order, resized, short, or claiming EOF early.
            elf(size, 2048),
            elf(size, 0),
            elf(size + 1, 1024),
            info(ELF, ELF_VERSION, size, 1024, 1000),
            Info {
                eof: true,
                ..elf(size, 1024)
            },
        ];
        for refused in refusals {
            let mut t = snapshot;
            assert_eq!(t.accept(&refused), Err(stage::PAIR_MISMATCH), "{refused:?}");
            assert_eq!(t, snapshot);
        }
        // An epoch that differs from the manifest's is refused on the first range.
        let mut fresh = transfer();
        let other = Info {
            retry_epoch: Epoch::new(EPOCH + 1).unwrap(),
            ..elf(size, 0)
        };
        assert_eq!(fresh.accept(&other), Err(stage::PAIR_MISMATCH));
        assert_eq!(fresh.size(), None);
    }

    #[test]
    fn image_size_follows_the_kernel_staging_bounds() {
        let max = runtime::MAX_STAGED_IMAGE_BYTES as u64;
        let min = runtime::MIN_STAGED_IMAGE_BYTES as u64;
        assert_eq!(transfer().accept(&elf(min - 1, 0)), Err(stage::IMAGE_SIZE));
        assert_eq!(transfer().accept(&elf(min, 0)).unwrap().begin, Some(min));
        assert_eq!(transfer().accept(&elf(max + 1, 0)), Err(stage::IMAGE_SIZE));
        assert_eq!(transfer().accept(&elf(max, 0)).unwrap().begin, Some(max));
        let empty = info(ELF, ELF_VERSION, 0, 0, 0);
        assert_eq!(transfer().accept(&empty), Err(stage::IMAGE_SIZE));
    }

    #[test]
    fn storage_policy_admits_the_file_server_requests_only() {
        // The shipped file-server manifest requests IPC and BLOCK.
        let file_server = application::IPC | application::BLOCK;
        assert_eq!(file_server & !STORAGE_FEATURES, 0);
        for refused in [
            application::DIAGNOSTIC,
            application::CONSOLE,
            application::CONTROL,
        ] {
            assert_ne!(refused & !STORAGE_FEATURES, 0);
        }
        assert_eq!(STORAGE_FEATURES & !application::KNOWN, 0);
    }

    #[test]
    fn refusal_statuses_stay_in_disjoint_ranges() {
        assert_eq!(file_refusal(FileError::Version), 32 + 13);
        assert_eq!(kernel_refusal(RuntimeError::Denied), 64);
        assert_eq!(kernel_refusal(RuntimeError::Protocol), 64 + 9);
        assert!(file_refusal(FileError::Unavailable) < stage::KERNEL_ERROR_BASE);
    }
}
