// SPDX-License-Identifier: Apache-2.0
//! Canonical bounded serialization for task replacement candidates.

use rustic_tasks_contract::{Document, MAX_BYTES, State};

const HEADER: &[u8] = b"rustic-tasks-v1\n";

/// Render a complete document and return its used length, or `None` on overflow.
pub(super) fn document(
    source: &Document,
    appended: Option<(u32, &[u8])>,
    done_id: Option<u32>,
    output: &mut [u8; MAX_BYTES],
) -> Option<usize> {
    let mut length = 0;
    if !push(output, &mut length, HEADER) {
        return None;
    }
    for index in 0..source.len() {
        let row = source.get(index).expect("document length bounds get");
        let state = if done_id == Some(row.id) {
            State::Done
        } else {
            row.state
        };
        if !write_task(output, &mut length, row.id, state, row.title()) {
            return None;
        }
    }
    if let Some((id, title)) = appended
        && !write_task(output, &mut length, id, State::Open, title)
    {
        return None;
    }
    Some(length)
}

fn write_task(
    output: &mut [u8; MAX_BYTES],
    length: &mut usize,
    id: u32,
    state: State,
    title: &[u8],
) -> bool {
    write_id(output, length, id)
        && push(output, length, b"\t")
        && push(
            output,
            length,
            match state {
                State::Open => b"open",
                State::Done => b"done",
            },
        )
        && push(output, length, b"\t")
        && push(output, length, title)
        && push(output, length, b"\n")
}

fn write_id(output: &mut [u8; MAX_BYTES], length: &mut usize, id: u32) -> bool {
    let mut digits = [0; 10];
    let mut count = 0;
    let mut value = id;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while count != 0 {
        count -= 1;
        if !push(output, length, &digits[count..count + 1]) {
            return false;
        }
    }
    true
}

fn push(output: &mut [u8], length: &mut usize, bytes: &[u8]) -> bool {
    let Some(end) = length.checked_add(bytes.len()) else {
        return false;
    };
    if end > output.len() {
        return false;
    }
    output[*length..end].copy_from_slice(bytes);
    *length = end;
    true
}
