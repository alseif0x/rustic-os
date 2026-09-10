// SPDX-License-Identifier: Apache-2.0
//! Line editing is pure state; overflow rejects the entire command, never a prefix.
pub const CAPACITY: usize = 1024;
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Inserted(u8),
    Erased,
    Cleared(usize),
    Line,
    Cancelled,
    Overflow,
    Invalid,
}
pub struct Editor {
    bytes: [u8; CAPACITY],
    len: usize,
    overflow: bool,
    invalid: bool,
    skip_lf: bool,
    escape: u8,
}
impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
impl Editor {
    pub const fn new() -> Self {
        Self {
            bytes: [0; CAPACITY],
            len: 0,
            overflow: false,
            invalid: false,
            skip_lf: false,
            escape: 0,
        }
    }
    pub fn reset(&mut self) {
        self.len = 0;
        self.overflow = false;
        self.invalid = false;
    }
    pub fn line(&mut self) -> &mut [u8] {
        &mut self.bytes[..self.len]
    }
    pub fn push(&mut self, b: u8) -> Event {
        if self.skip_lf {
            self.skip_lf = false;
            if b == b'\n' {
                return Event::None;
            }
        }
        if self.escape != 0 {
            if self.escape == 1 && b == b'[' {
                self.escape = 2;
            } else if self.escape == 1 || (0x40..=0x7e).contains(&b) {
                self.escape = 0;
            }
            return Event::None;
        }
        match b {
            27 => {
                self.escape = 1;
                Event::None
            }
            b'\r' | b'\n' => {
                self.skip_lf = b == b'\r';
                if self.invalid {
                    Event::Invalid
                } else if self.overflow {
                    Event::Overflow
                } else {
                    Event::Line
                }
            }
            3 => {
                self.reset();
                Event::Cancelled
            }
            21 => {
                let n = self.len;
                self.reset();
                Event::Cleared(n)
            }
            8 | 127 => {
                if self.len > 0 {
                    self.len -= 1;
                    Event::Erased
                } else {
                    Event::None
                }
            }
            b'\t' | 32..=126 => {
                if self.len == CAPACITY {
                    self.overflow = true;
                    Event::None
                } else {
                    let b = if b == b'\t' { b' ' } else { b };
                    self.bytes[self.len] = b;
                    self.len += 1;
                    Event::Inserted(b)
                }
            }
            _ => {
                self.invalid = true;
                Event::None
            }
        }
    }
}
