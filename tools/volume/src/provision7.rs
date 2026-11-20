// SPDX-License-Identifier: Apache-2.0
//! Fresh owner-managed images, with policy present before the guest mounts.
use std::path::Path;

use rustic_fs::{Kind, Volume7};

use crate::command::{V7_IMAGE_SECTORS, parse_lineage, report7};
use crate::disk::FileDisk;

const POLICY: &[u8] = b"rustic-owner-v1\nhelpers=explicit\n";

pub(crate) fn provision7(image: &Path, lineage: &str) -> Result<String, String> {
    let lineage = parse_lineage(lineage)?;
    let mut disk = FileDisk::create_new(image, V7_IMAGE_SECTORS)?;
    let mut volume = Volume7::EMPTY;
    volume
        .provision_into(&mut disk, lineage)
        .map_err(|error| format!("v7 provision refused: {error:?}"))?;
    let policy = volume
        .create(&mut disk, 3, b"owner-policy", Kind::File)
        .map_err(|error| format!("owner policy creation refused: {error:?}"))?;
    volume
        .replace(&mut disk, policy.id, policy.version, POLICY)
        .map_err(|error| format!("owner policy write refused: {error:?}"))?;
    // Ordinary provisioning does not occupy retry records or consume a subject.
    drop(disk);
    report7(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const LINEAGE: &str = "0112233445566778899aabbccddeeff0";

    #[test]
    fn fresh_owner_image_has_policy_and_no_retry_history() {
        let directory = TempDir::new();
        let image = directory.path().join("owner.raw");
        provision7(&image, LINEAGE).unwrap();
        let mut disk = FileDisk::open_read_only(&image).unwrap();
        let mut volume = Volume7::EMPTY;
        volume.mount_into(&mut disk).unwrap();
        assert_eq!(
            volume
                .nodes()
                .unwrap()
                .iter()
                .filter(|n| n.kind != Kind::Empty)
                .count(),
            5
        );
        assert!(
            volume
                .retained_records()
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
        let policy = volume.lookup(3, b"owner-policy").unwrap();
        let mut bytes = [0; 64];
        let length = volume
            .read_range(&mut disk, policy.id, Some(policy.version), 0, &mut bytes)
            .unwrap();
        assert_eq!(&bytes[..length], POLICY);
    }

    #[test]
    fn refuses_existing_path_and_invalid_lineage_without_touching_inputs() {
        let directory = TempDir::new();
        let image = directory.path().join("existing.raw");
        std::fs::write(&image, b"owner bytes").unwrap();
        assert!(provision7(&image, LINEAGE).is_err());
        assert_eq!(std::fs::read(&image).unwrap(), b"owner bytes");
        let absent = directory.path().join("absent.raw");
        assert!(provision7(&absent, "invalid").is_err());
        assert!(!absent.exists());
    }
}
