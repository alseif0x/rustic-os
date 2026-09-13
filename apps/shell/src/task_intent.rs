// SPDX-License-Identifier: Apache-2.0
//! Owner-client recovery record. It stores native app output, not edit policy.
use rustic_sdk::abi::files::{
    Error,
    operation::{Key, Operation, Replacement, Retry},
    reference::{Epoch, References, Version},
};
use rustic_tasks_contract::{Document, preview::Edit};
use sha2::{Digest, Sha256};

const HEADER: usize = 136;
const MAGIC: &[u8; 8] = b"RTSKI001";
const _: () = assert!(
    HEADER
        + b"rustic-tasks-v1\n".len()
        + rustic_tasks_contract::MAX_TASKS
            * (10 + 1 + 4 + 1 + rustic_tasks_contract::MAX_TITLE + 1)
        <= 1024
);

/// The bytes become immutable before submission. The journal's committed
/// metadata version, allocated by the volume, supplies its instance/key.
pub struct Intent {
    bytes: [u8; 1024],
    length: usize,
}
impl Intent {
    pub fn new(
        refs: References,
        version: Version,
        epoch: Epoch,
        edit: Edit,
        task_id: u32,
        candidate: &[u8],
    ) -> Result<Self, Error> {
        if refs.workspace != refs.resource.workspace()
            || task_id == 0
            || Edit::decode(edit.words()) != Some(edit)
            || candidate.len() + HEADER > 1024
            || Document::parse(candidate).is_err()
        {
            return Err(Error::Invalid);
        }
        let mut value = Self {
            bytes: [0; 1024],
            length: HEADER + candidate.len(),
        };
        let b = &mut value.bytes;
        b[..8].copy_from_slice(MAGIC);
        b[8..24].copy_from_slice(&refs.workspace.lineage());
        b[24..28].copy_from_slice(&refs.workspace.root().to_le_bytes());
        b[28..32].copy_from_slice(&refs.resource.object().to_le_bytes());
        b[32..40].copy_from_slice(&version.value().to_le_bytes());
        b[40..48].copy_from_slice(&epoch.value().to_le_bytes());
        for (chunk, word) in b[48..96]
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip(edit.words())
        {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        b[96..100].copy_from_slice(&task_id.to_le_bytes());
        b[100..102].copy_from_slice(&(candidate.len() as u16).to_le_bytes());
        b[104..136].copy_from_slice(&Sha256::digest(candidate));
        b[HEADER..value.length].copy_from_slice(candidate);
        Ok(value)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER
            || bytes.len() > 1024
            || &bytes[..8] != MAGIC
            || bytes[102..104] != [0; 2]
        {
            return Err(Error::Invalid);
        }
        let number = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let refs = References::new(
            bytes[8..24].try_into().unwrap(),
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
        )?;
        let mut words = [0; 6];
        for (index, word) in words.iter_mut().enumerate() {
            *word = number(48 + index * 8);
        }
        let result = Self::new(
            refs,
            Version::new(number(32))?,
            Epoch::new(number(40))?,
            Edit::decode(words).ok_or(Error::Invalid)?,
            u32::from_le_bytes(bytes[96..100].try_into().unwrap()),
            &bytes[HEADER..],
        )?;
        if result.encoded() != bytes {
            return Err(Error::Invalid);
        }
        Ok(result)
    }
    pub fn encoded(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
    pub fn candidate(&self) -> &[u8] {
        &self.bytes[HEADER..self.length]
    }
    pub fn task_id(&self) -> u32 {
        u32::from_le_bytes(self.bytes[96..100].try_into().unwrap())
    }
    pub fn request(&self, journal_version: u64) -> Result<Replacement, Error> {
        let b = &self.bytes;
        let number = |offset| u64::from_le_bytes(b[offset..offset + 8].try_into().unwrap());
        let refs = References::new(
            b[8..24].try_into().unwrap(),
            u32::from_le_bytes(b[24..28].try_into().unwrap()),
            u32::from_le_bytes(b[28..32].try_into().unwrap()),
        )?;
        if journal_version <= number(32) {
            return Err(Error::Invalid);
        }
        Ok(Replacement {
            workspace: refs.workspace,
            resource: refs.resource,
            expected_version: Version::new(number(32))?,
            retry: Retry {
                epoch: Epoch::new(number(40))?,
                key: Key::new(journal_version)?,
            },
        })
    }
    /// An older owner operation that happens to use this numeric key cannot
    /// prove a new journal instance. Content equality alone is insufficient.
    pub fn matches(&self, key: u64, operation: &Operation) -> bool {
        let Ok(request) = self.request(key) else {
            return false;
        };
        operation.encode().is_ok()
            && operation.workspace == request.workspace
            && operation.resource == request.resource
            && operation.retry == request.retry
            && operation.previous_version == request.expected_version
            && operation.version.value() > key
            && operation.id.sequence() == operation.version.value()
            && operation.size as usize == self.candidate().len()
            && operation.sha256.as_slice() == &self.bytes[104..136]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn intent() -> Intent {
        Intent::new(
            References::new([1; 16], 4, 9).unwrap(),
            Version::new(7).unwrap(),
            Epoch::new(1).unwrap(),
            Edit::Done { id: 1 },
            1,
            b"rustic-tasks-v1\n1\tdone\tFirst\n",
        )
        .unwrap()
    }
    #[test]
    fn retained_bytes_and_identity_survive_decode() {
        let original = intent();
        let restored = Intent::decode(original.encoded()).unwrap();
        assert_eq!(restored.request(10).unwrap(), original.request(10).unwrap());
        assert_eq!(restored.candidate(), original.candidate());
        assert!(restored.request(7).is_err());
        for index in 0..original.encoded().len() {
            let mut changed = original.bytes;
            changed[index] ^= 0x80;
            // Candidate hash and canonical framing are checked; changing valid
            // identity fields is an owner edit, not authenticated provenance.
            if !(8..100).contains(&index) {
                assert!(Intent::decode(&changed[..original.length]).is_err());
            }
        }
    }
    #[test]
    fn old_or_mismatched_receipt_never_proves_new_intent() {
        use rustic_sdk::abi::files::operation::{Instance, OperationId};
        let intent = intent();
        let request = intent.request(10).unwrap();
        let mut operation = Operation {
            id: OperationId::new([1; 16], 11).unwrap(),
            service_instance: Instance::new([1; 16], 8).unwrap(),
            workspace: request.workspace,
            resource: request.resource,
            previous_version: request.expected_version,
            version: Version::new(11).unwrap(),
            retry: request.retry,
            size: intent.candidate().len() as u16,
            sha256: Sha256::digest(intent.candidate()).into(),
        };
        assert!(intent.matches(10, &operation));
        operation.version = Version::new(9).unwrap();
        operation.id = OperationId::new([1; 16], 9).unwrap();
        assert!(!intent.matches(10, &operation));
        operation.version = Version::new(11).unwrap();
        operation.id = OperationId::new([1; 16], 11).unwrap();
        operation.sha256[0] ^= 1;
        assert!(!intent.matches(10, &operation));
    }
}
