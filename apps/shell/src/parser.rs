// SPDX-License-Identifier: Apache-2.0
//! Bounded in-place quoting and escaping. Parse errors have no command effects.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Quote,
    Escape,
    Arguments,
    Encoding,
}
pub struct Args<'a> {
    text: &'a [u8],
    ranges: [(usize, usize); 16],
    len: usize,
}
impl<'a> Args<'a> {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, n: usize) -> Option<&'a str> {
        if n >= self.len {
            return None;
        }
        let (a, b) = self.ranges[n];
        core::str::from_utf8(&self.text[a..b]).ok()
    }
}
pub fn parse(bytes: &mut [u8]) -> Result<Args<'_>, Error> {
    if !bytes.is_ascii() {
        return Err(Error::Encoding);
    }
    let mut ranges = [(0, 0); 16];
    let (mut read, mut write, mut len) = (0, 0, 0);
    while read < bytes.len() {
        while read < bytes.len() && bytes[read] == b' ' {
            read += 1;
        }
        if read == bytes.len() {
            break;
        }
        if len == ranges.len() {
            return Err(Error::Arguments);
        }
        let start = write;
        let mut quote = 0;
        while read < bytes.len() {
            let b = bytes[read];
            read += 1;
            if quote == 0 && b == b' ' {
                break;
            }
            if b == b'\'' || b == b'"' {
                if quote == 0 {
                    quote = b;
                    continue;
                }
                if quote == b {
                    quote = 0;
                    continue;
                }
            }
            if b == b'\\' && quote != b'\'' {
                if read == bytes.len() {
                    return Err(Error::Escape);
                }
                bytes[write] = match bytes[read] {
                    b'n' => b'\n',
                    b't' => b'\t',
                    c => c,
                };
                read += 1;
            } else {
                bytes[write] = b;
            }
            write += 1;
        }
        if quote != 0 {
            return Err(Error::Quote);
        }
        ranges[len] = (start, write);
        len += 1;
    }
    Ok(Args {
        text: &bytes[..write],
        ranges,
        len,
    })
}
