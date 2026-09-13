// SPDX-License-Identifier: Apache-2.0
//! Fixed words exchanged by the tasks child and its supervisor.

use crate::{MAX_TITLE, State, Task};

pub const LIST: u64 = 1;
pub const NEXT: u64 = 2;

pub const ROW: u64 = 0;
pub const END: u64 = 1;
pub const INVALID: u64 = 2;
pub const CAPACITY: u64 = 3;
pub const SERVICE: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    List,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub state: State,
    pub title: [u8; MAX_TITLE],
    pub title_len: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    Row(Row),
    End { count: u32 },
    Invalid,
    Capacity,
    Service { code: u64 },
}

pub fn decode_request(words: [u64; 8]) -> Option<Request> {
    if words[2..].iter().any(|word| *word != 0) {
        return None;
    }
    match words[0] {
        LIST => (words[1] == 0).then_some(Request::List),
        NEXT => (words[1] == 0).then_some(Request::Next),
        _ => None,
    }
}

pub fn row(task: &Task) -> [u64; 8] {
    row_words(&Row {
        id: task.id,
        state: task.state,
        title: task.title,
        title_len: task.title_len,
    })
}

pub fn row_words(row: &Row) -> [u64; 8] {
    [
        ROW,
        u64::from(row.id),
        row.state.code(),
        u64::from(row.title_len),
        u64::from_le_bytes(row.title[..8].try_into().unwrap()),
        u64::from_le_bytes(row.title[8..16].try_into().unwrap()),
        u64::from_le_bytes(row.title[16..24].try_into().unwrap()),
        0,
    ]
}

pub const fn end(count: usize) -> [u64; 8] {
    [END, count as u64, 0, 0, 0, 0, 0, 0]
}

pub const fn invalid() -> [u64; 8] {
    [INVALID, 0, 0, 0, 0, 0, 0, 0]
}

pub const fn capacity() -> [u64; 8] {
    [CAPACITY, 0, 0, 0, 0, 0, 0, 0]
}

pub const fn service(code: u64) -> [u64; 8] {
    [SERVICE, code, 0, 0, 0, 0, 0, 0]
}

pub fn decode_response(words: [u64; 8]) -> Option<Response> {
    match words[0] {
        ROW => {
            let title_len = usize::try_from(words[3]).ok()?;
            if words[1] == 0 || title_len == 0 || title_len > MAX_TITLE {
                return None;
            }
            let state = State::from_code(words[2])?;
            let mut title = [0; MAX_TITLE];
            for (chunk, word) in title.as_chunks_mut::<8>().0.iter_mut().zip(&words[4..7]) {
                chunk.copy_from_slice(&word.to_le_bytes());
            }
            if title[title_len..].iter().any(|byte| *byte != 0)
                || title[..title_len]
                    .iter()
                    .any(|byte| !(0x20..=0x7e).contains(byte))
                || words[7] != 0
            {
                return None;
            }
            Some(Response::Row(Row {
                id: u32::try_from(words[1]).ok()?,
                state,
                title,
                title_len: title_len as u8,
            }))
        }
        END if words[2..].iter().all(|word| *word == 0)
            && usize::try_from(words[1]).ok()? <= crate::MAX_TASKS =>
        {
            Some(Response::End {
                count: u32::try_from(words[1]).ok()?,
            })
        }
        INVALID if words[1..].iter().all(|word| *word == 0) => Some(Response::Invalid),
        CAPACITY if words[1..].iter().all(|word| *word == 0) => Some(Response::Capacity),
        SERVICE if words[2..].iter().all(|word| *word == 0) => {
            Some(Response::Service { code: words[1] })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn request_and_row_codec_reject_reserved_words() {
        assert_eq!(
            decode_request([LIST, 0, 0, 0, 0, 0, 0, 0]),
            Some(Request::List)
        );
        assert_eq!(decode_request([LIST, 1, 0, 0, 0, 0, 0, 0]), None);
        let document = Document::parse(b"rustic-tasks-v1\n7\tdone\tBoot\n").unwrap();
        let encoded = row(document.get(0).unwrap());
        assert_eq!(
            decode_response(encoded),
            Some(Response::Row(Row {
                id: 7,
                state: State::Done,
                title: {
                    let mut title = [0; MAX_TITLE];
                    title[..4].copy_from_slice(b"Boot");
                    title
                },
                title_len: 4,
            }))
        );
        let mut bad = encoded;
        bad[7] = 1;
        assert_eq!(decode_response(bad), None);
    }

    #[test]
    fn error_and_end_codecs_are_canonical() {
        assert_eq!(decode_response(end(16)), Some(Response::End { count: 16 }));
        assert_eq!(decode_response(invalid()), Some(Response::Invalid));
        assert_eq!(decode_response(capacity()), Some(Response::Capacity));
        assert_eq!(
            decode_response(service(12)),
            Some(Response::Service { code: 12 })
        );
        let mut bad = end(1);
        bad[7] = 1;
        assert_eq!(decode_response(bad), None);
    }
}
