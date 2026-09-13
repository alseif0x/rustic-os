// SPDX-License-Identifier: Apache-2.0
//! Complete, bounded task document parsing.

pub const HEADER: &[u8] = b"rustic-tasks-v1\n";
pub const MAX_BYTES: usize = 1024;
pub const MAX_TASKS: usize = 16;
pub const MAX_TITLE: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Capacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Done,
}

impl State {
    pub const fn code(self) -> u64 {
        match self {
            Self::Open => 1,
            Self::Done => 2,
        }
    }

    pub const fn from_code(code: u64) -> Option<Self> {
        match code {
            1 => Some(Self::Open),
            2 => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: u32,
    pub state: State,
    pub title: [u8; MAX_TITLE],
    pub title_len: u8,
}

impl Task {
    const EMPTY: Self = Self {
        id: 0,
        state: State::Open,
        title: [0; MAX_TITLE],
        title_len: 0,
    };

    pub fn title(&self) -> &[u8] {
        &self.title[..usize::from(self.title_len)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Document {
    tasks: [Task; MAX_TASKS],
    len: usize,
}

impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_BYTES || !bytes.starts_with(HEADER) {
            return Err(Error::Invalid);
        }
        let mut document = Self {
            tasks: [Task::EMPTY; MAX_TASKS],
            len: 0,
        };
        let mut offset = HEADER.len();
        while offset < bytes.len() {
            let rest = &bytes[offset..];
            let end = rest
                .iter()
                .position(|b| *b == b'\n')
                .ok_or(Error::Invalid)?;
            let line = &rest[..end];
            offset += end + 1;
            document.push(parse_line(line)?)?;
        }
        Ok(document)
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<&Task> {
        self.tasks.get(index).filter(|_| index < self.len)
    }

    fn push(&mut self, task: Task) -> Result<(), Error> {
        if self.len == MAX_TASKS {
            return Err(Error::Capacity);
        }
        if self.tasks[..self.len].iter().any(|old| old.id == task.id) {
            return Err(Error::Invalid);
        }
        self.tasks[self.len] = task;
        self.len += 1;
        Ok(())
    }
}

fn parse_line(line: &[u8]) -> Result<Task, Error> {
    let first = line
        .iter()
        .position(|b| *b == b'\t')
        .ok_or(Error::Invalid)?;
    let after_id = &line[first + 1..];
    let second = after_id
        .iter()
        .position(|b| *b == b'\t')
        .ok_or(Error::Invalid)?;
    let state = match &after_id[..second] {
        b"open" => State::Open,
        b"done" => State::Done,
        _ => return Err(Error::Invalid),
    };
    let title = &after_id[second + 1..];
    if title.is_empty() {
        return Err(Error::Invalid);
    }
    if title.len() > MAX_TITLE {
        return Err(Error::Capacity);
    }
    if title.iter().any(|b| !(0x20..=0x7e).contains(b)) {
        return Err(Error::Invalid);
    }
    let id = parse_id(&line[..first])?;
    let mut stored = [0; MAX_TITLE];
    stored[..title.len()].copy_from_slice(title);
    Ok(Task {
        id,
        state,
        title: stored,
        title_len: title.len() as u8,
    })
}

fn parse_id(bytes: &[u8]) -> Result<u32, Error> {
    if bytes.is_empty() || bytes.len() > 10 || bytes[0] == b'0' {
        return Err(Error::Invalid);
    }
    let mut value = 0u32;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return Err(Error::Invalid);
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            .ok_or(Error::Invalid)?;
    }
    if value == 0 {
        return Err(Error::Invalid);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{format, string::String};

    fn parse(text: &str) -> Result<Document, Error> {
        Document::parse(text.as_bytes())
    }

    #[test]
    fn accepts_header_only_and_preserves_order_and_u32_max() {
        assert_eq!(parse("rustic-tasks-v1\n").unwrap().len(), 0);
        let document =
            parse("rustic-tasks-v1\n7\topen\tReview kernel\n4294967295\tdone\tBoot the OS\n")
                .unwrap();
        assert_eq!(document.len(), 2);
        assert_eq!(document.get(0).unwrap().id, 7);
        assert_eq!(document.get(0).unwrap().state, State::Open);
        assert_eq!(document.get(1).unwrap().id, u32::MAX);
        assert_eq!(document.get(1).unwrap().title(), b"Boot the OS");
    }

    #[test]
    fn rejects_duplicate_ids_and_malformed_tail_before_rows() {
        for text in [
            "rustic-tasks-v1\n7\topen\tone\n7\tdone\ttwo\n",
            "rustic-tasks-v1\n7\topen\tone\nmalformed tail\n",
            "rustic-tasks-v1\n7\topen\tone",
            "rustic-tasks-v2\n",
        ] {
            assert_eq!(parse(text), Err(Error::Invalid), "{text:?}");
        }
    }

    #[test]
    fn distinguishes_capacity_from_invalid_content() {
        let rows = (1..=17)
            .map(|id| format!("{id}\topen\tx\n"))
            .collect::<String>();
        assert_eq!(
            parse(&format!("rustic-tasks-v1\n{rows}")),
            Err(Error::Capacity)
        );
        assert_eq!(
            parse("rustic-tasks-v1\n1\topen\t1234567890123456789012345\n"),
            Err(Error::Capacity)
        );
        assert_eq!(
            parse("rustic-tasks-v1\n1\topen\tbad\tseparator\n"),
            Err(Error::Invalid)
        );
    }
}
