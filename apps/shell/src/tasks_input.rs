// SPDX-License-Identifier: Apache-2.0
//! Bounded hexadecimal entry of one candidate chunk. Parsing has no effect: a
//! refusal here means no request was ever submitted.
use rustic_tasks_contract::candidate;

/// Decode up to [`candidate::MAX_CHUNK`] bytes of hexadecimal into the payload
/// words of one owner chunk request: `[length, b0, b1, b2, b3]`.
///
/// The byte order is the product codec's; this module packs no words of its
/// own. `None` means the text is empty, of odd length, longer than the bounded
/// chunk, or not hexadecimal.
pub fn chunk_words(text: &str) -> Option<[u64; 5]> {
    let text = text.as_bytes();
    if text.is_empty() || !text.len().is_multiple_of(2) || text.len() > 2 * candidate::MAX_CHUNK {
        return None;
    }
    let mut bytes = [0; candidate::MAX_CHUNK];
    for (byte, pair) in bytes.iter_mut().zip(text.as_chunks::<2>().0) {
        *byte = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    let w = candidate::owner_words(&candidate::chunk(&bytes[..text.len() / 2], 0)?);
    Some([w[3], w[4], w[5], w[6], w[7]])
}

fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(words: [u64; 5]) -> candidate::Chunk {
        candidate::decode_owner([
            0, 0, words[0], words[0], words[1], words[2], words[3], words[4],
        ])
        .unwrap()
    }

    #[test]
    fn hex_round_trips_through_the_product_codec() {
        let words = chunk_words("48656c6c6f").unwrap();
        assert_eq!(words[0], 5);
        assert_eq!(decode(words).bytes(), b"Hello");
        // Case is irrelevant, and both digits of a byte are combined in order.
        assert_eq!(chunk_words("0A1b"), chunk_words("0a1B"));
        assert_eq!(decode(chunk_words("0A1b").unwrap()).bytes(), &[0x0a, 0x1b]);
    }

    #[test]
    fn a_full_chunk_is_accepted_and_a_longer_one_is_not() {
        let mut full = [0; 2 * candidate::MAX_CHUNK];
        for pair in full.as_chunks_mut::<2>().0 {
            pair.copy_from_slice(b"ab");
        }
        let text = core::str::from_utf8(&full).unwrap();
        let words = chunk_words(text).unwrap();
        assert_eq!(words[0], candidate::MAX_CHUNK as u64);
        let expected = [0xab_u8; candidate::MAX_CHUNK];
        assert_eq!(decode(words).bytes(), &expected[..]);
        let mut over = [b'a'; 2 * candidate::MAX_CHUNK + 2];
        over[..full.len()].copy_from_slice(&full);
        assert_eq!(chunk_words(core::str::from_utf8(&over).unwrap()), None);
    }

    #[test]
    fn empty_odd_and_nonhexadecimal_text_is_refused() {
        assert_eq!(chunk_words(""), None);
        assert_eq!(chunk_words("abc"), None);
        assert_eq!(chunk_words("zz"), None);
        assert_eq!(chunk_words("0x"), None);
        assert_eq!(chunk_words(" 0a"), None);
    }
}
