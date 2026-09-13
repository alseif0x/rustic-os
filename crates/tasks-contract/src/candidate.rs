// SPDX-License-Identifier: Apache-2.0
//! Bounded byte transport for one immutable task-edit candidate.
//!
//! A chunk always occupies one eight-word message. The first word is kept
//! separate for the private child response and the owner response: the child
//! uses [`CHUNK`], while an owner reply uses the ordinary success value zero.
//! The remaining words are offset, total length, payload length and four
//! little-endian payload words. Bytes after `length` are required to be zero.

use crate::MAX_BYTES;

/// Child request asking for the candidate bytes beginning at an offset.
pub const BYTES: u64 = 4;
/// Private child response carrying one candidate chunk.
pub const CHUNK: u64 = 6;
/// Alias naming the child response in terms of the product contract.
pub const CANDIDATE: u64 = CHUNK;
/// Number of payload bytes carried by one fixed-size message.
pub const MAX_CHUNK: usize = 32;

/// One canonical bounded candidate chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub offset: u64,
    pub total: u64,
    pub length: u64,
    pub bytes: [u8; MAX_CHUNK],
}

impl Chunk {
    /// Return the initialized payload bytes in this chunk.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::try_from(self.length).unwrap()]
    }

    /// Return whether this chunk reaches the candidate's declared end.
    pub const fn is_final(&self) -> bool {
        match self.offset.checked_add(self.length) {
            Some(end) => end == self.total,
            None => false,
        }
    }

    /// Decode and validate a private child chunk response.
    pub fn decode_response(words: [u64; 8]) -> Option<Self> {
        decode_words(words, CHUNK)
    }

    /// Decode and validate an owner success response carrying a candidate chunk.
    pub fn decode_owner(words: [u64; 8]) -> Option<Self> {
        decode_words(words, 0)
    }
}

/// Encode a private child byte request.
pub const fn request_words(offset: usize) -> [u64; 8] {
    [BYTES, offset as u64, 0, 0, 0, 0, 0, 0]
}

/// Decode and validate a private child byte request.
pub fn decode_request(words: [u64; 8]) -> Option<usize> {
    if words[0] != BYTES || words[2..].iter().any(|word| *word != 0) {
        return None;
    }
    let offset = usize::try_from(words[1]).ok()?;
    (offset <= MAX_BYTES).then_some(offset)
}

/// Build the canonical chunk at `offset` from a complete candidate.
pub fn chunk(candidate: &[u8], offset: usize) -> Option<Chunk> {
    if candidate.is_empty() || candidate.len() > MAX_BYTES || offset >= candidate.len() {
        return None;
    }
    let length = core::cmp::min(MAX_CHUNK, candidate.len() - offset);
    let mut bytes = [0; MAX_CHUNK];
    bytes[..length].copy_from_slice(&candidate[offset..offset + length]);
    Some(Chunk {
        offset: offset as u64,
        total: candidate.len() as u64,
        length: length as u64,
        bytes,
    })
}

/// Encode a private child chunk response.
pub fn response_words(chunk: &Chunk) -> [u64; 8] {
    words(CHUNK, chunk)
}

/// Encode an owner success response carrying the same pure chunk fields.
pub fn owner_words(chunk: &Chunk) -> [u64; 8] {
    words(0, chunk)
}

/// Decode and validate a private child chunk response.
pub fn decode_response(words: [u64; 8]) -> Option<Chunk> {
    Chunk::decode_response(words)
}

/// Decode and validate an owner success response carrying a candidate chunk.
pub fn decode_owner(words: [u64; 8]) -> Option<Chunk> {
    Chunk::decode_owner(words)
}

fn words(marker: u64, chunk: &Chunk) -> [u64; 8] {
    [
        marker,
        chunk.offset,
        chunk.total,
        chunk.length,
        u64::from_le_bytes(chunk.bytes[..8].try_into().unwrap()),
        u64::from_le_bytes(chunk.bytes[8..16].try_into().unwrap()),
        u64::from_le_bytes(chunk.bytes[16..24].try_into().unwrap()),
        u64::from_le_bytes(chunk.bytes[24..32].try_into().unwrap()),
    ]
}

fn decode_words(words: [u64; 8], marker: u64) -> Option<Chunk> {
    if words[0] != marker {
        return None;
    }
    let offset = words[1];
    let total = words[2];
    let length = words[3];
    if total == 0
        || total > MAX_BYTES as u64
        || offset >= total
        || length == 0
        || length > MAX_CHUNK as u64
    {
        return None;
    }
    let remaining = total - offset;
    if length != core::cmp::min(MAX_CHUNK as u64, remaining) {
        return None;
    }
    let mut bytes = [0; MAX_CHUNK];
    for (chunk, word) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(&words[4..]) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    let length = usize::try_from(length).ok()?;
    if bytes[length..].iter().any(|byte| *byte != 0) {
        return None;
    }
    Some(Chunk {
        offset,
        total,
        length: length as u64,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_are_fixed_width_and_zero_padded() {
        let bytes = [b'a'; 33];
        let first = chunk(&bytes, 0).unwrap();
        assert_eq!(first.length, 32);
        assert!(!first.is_final());
        assert_eq!(decode_response(response_words(&first)), Some(first));
        assert_eq!(decode_owner(owner_words(&first)), Some(first));

        let last = chunk(&bytes, 32).unwrap();
        assert_eq!(last.length, 1);
        assert!(last.is_final());
        assert_eq!(decode_response(response_words(&last)), Some(last));
        assert!(last.bytes[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn requests_and_chunks_reject_bounds_progress_and_padding_violations() {
        assert_eq!(decode_request(request_words(0)), Some(0));
        assert_eq!(decode_request(request_words(MAX_BYTES)), Some(MAX_BYTES));
        assert_eq!(decode_request(request_words(MAX_BYTES + 1)), None);

        let bytes = [b'x'; 33];
        let valid = response_words(&chunk(&bytes, 0).unwrap());
        for (index, value) in [(1, 2), (2, MAX_BYTES as u64 + 1), (3, 31)] {
            let mut malformed = valid;
            malformed[index] = value;
            assert_eq!(decode_response(malformed), None, "word {index}");
        }
        let last = response_words(&chunk(&bytes, 32).unwrap());
        let mut bad_padding = last;
        bad_padding[5] = 1;
        assert_eq!(decode_response(bad_padding), None);
        let mut wrong_marker = valid;
        wrong_marker[0] = 0;
        assert_eq!(decode_response(wrong_marker), None);
    }

    #[test]
    fn arbitrary_offsets_use_the_remaining_bounded_payload() {
        let bytes = [b'z'; 40];
        let part = chunk(&bytes, 9).unwrap();
        assert_eq!(part.length, 31);
        assert!(part.is_final());
        assert_eq!(decode_response(response_words(&part)), Some(part));
        assert!(chunk(&bytes, 40).is_none());
        assert!(chunk(&[], 0).is_none());
    }
}
