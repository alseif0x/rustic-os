// SPDX-License-Identifier: Apache-2.0
//! Read-only edit preview transport. No commit or replay authority is encoded.
use crate::{MAX_TASKS, MAX_TITLE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Add { title: [u8; MAX_TITLE], length: u8 },
    Done { id: u32 },
}

impl Edit {
    pub fn add(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty()
            || bytes.len() > MAX_TITLE
            || bytes.iter().any(|b| !(0x20..=0x7e).contains(b))
        {
            return None;
        }
        let mut title = [0; MAX_TITLE];
        title[..bytes.len()].copy_from_slice(bytes);
        Some(Self::Add {
            title,
            length: bytes.len() as u8,
        })
    }

    pub fn words(self) -> [u64; 6] {
        match self {
            Self::Done { id } => [2, u64::from(id), 0, 0, 0, 0],
            Self::Add { title, length } => [
                1,
                u64::from(length),
                u64::from_le_bytes(title[..8].try_into().unwrap()),
                u64::from_le_bytes(title[8..16].try_into().unwrap()),
                u64::from_le_bytes(title[16..].try_into().unwrap()),
                0,
            ],
        }
    }

    pub fn decode(words: [u64; 6]) -> Option<Self> {
        match words[0] {
            1 if (1..=MAX_TITLE as u64).contains(&words[1]) && words[5] == 0 => {
                let mut title = [0; MAX_TITLE];
                for (chunk, word) in title.as_chunks_mut::<8>().0.iter_mut().zip(&words[2..5]) {
                    chunk.copy_from_slice(&word.to_le_bytes());
                }
                let length = words[1] as usize;
                if title[length..].iter().any(|b| *b != 0) {
                    return None;
                }
                Self::add(&title[..length])
            }
            2 if words[1] != 0 && words[2..].iter().all(|word| *word == 0) => Some(Self::Done {
                id: u32::try_from(words[1]).ok()?,
            }),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub count: u32,
    pub version: u64,
    pub task_id: u32,
    pub changed: bool,
}

impl Summary {
    pub fn decode(words: [u64; 7]) -> Option<Self> {
        if words[0] > MAX_TASKS as u64
            || words[1] == 0
            || words[2] == 0
            || words[3] > 1
            || words[4..].iter().any(|word| *word != 0)
        {
            return None;
        }
        Some(Self {
            count: u32::try_from(words[0]).ok()?,
            version: words[1],
            task_id: u32::try_from(words[2]).ok()?,
            changed: words[3] == 1,
        })
    }
    pub const fn words(self) -> [u64; 7] {
        [
            self.count as u64,
            self.version,
            self.task_id as u64,
            self.changed as u64,
            0,
            0,
            0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_wire_rejects_noncanonical_payloads() {
        for edit in [
            Edit::add(b"Review Rust").unwrap(),
            Edit::Done { id: u32::MAX },
        ] {
            assert_eq!(Edit::decode(edit.words()), Some(edit));
            let mut bad = edit.words();
            bad[5] = 1;
            assert_eq!(Edit::decode(bad), None);
        }
        assert_eq!(Edit::decode([2, 0, 0, 0, 0, 0]), None);
        assert_eq!(Edit::decode([1, 1, 0x6120, 0, 0, 0]), None);
        assert_eq!(Edit::add(b"bad\ttitle"), None);
    }
    #[test]
    fn summary_requires_version_identity_and_boolean() {
        let summary = Summary {
            count: 2,
            version: 9,
            task_id: 42,
            changed: false,
        };
        assert_eq!(Summary::decode(summary.words()), Some(summary));
        for (index, value) in [(0, 17), (1, 0), (2, 0), (3, 2), (6, 1)] {
            let mut bad = summary.words();
            bad[index] = value;
            assert_eq!(Summary::decode(bad), None);
        }
    }
}
