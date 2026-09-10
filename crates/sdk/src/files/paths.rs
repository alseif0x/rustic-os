// SPDX-License-Identifier: Apache-2.0
use super::{Client, Error};
impl Client {
    pub fn resolve(&mut self, cwd: u32, path: &str) -> Result<u32, Error> {
        if path.is_empty() || path.len() > 255 {
            return Err(Error::Invalid);
        }
        let mut id = if path.starts_with('/') { 0 } else { cwd };
        for name in path.split('/') {
            if !name.is_empty() && id != 0 && !self.stat(id)?.directory {
                return Err(Error::NotDirectory);
            }
            match name {
                "" | "." => {}
                ".." => {
                    if id != 0 {
                        id = self.stat(id)?.parent;
                    }
                }
                _ => {
                    id = self.lookup(id, name)?.id;
                }
            }
        }
        Ok(id)
    }
    pub fn parent<'a>(&mut self, cwd: u32, path: &'a str) -> Result<(u32, &'a str), Error> {
        if path.is_empty() || path.ends_with('/') {
            return Err(Error::Invalid);
        }
        let (base, name) = path.rsplit_once('/').unwrap_or((".", path));
        if name == "." || name == ".." {
            return Err(Error::Invalid);
        }
        let parent = if base.is_empty() {
            0
        } else {
            self.resolve(cwd, base)?
        };
        Ok((parent, name))
    }
    pub fn path(&mut self, id: u32, bytes: &mut [u8; 256]) -> Result<usize, Error> {
        let mut current = id;
        let mut end = bytes.len();
        for _ in 0..32 {
            if current == 0 {
                if end == bytes.len() {
                    end -= 1;
                    bytes[end] = b'/';
                }
                let n = bytes.len() - end;
                bytes.copy_within(end.., 0);
                return Ok(n);
            }
            let m = self.stat(current)?;
            let name = m.name().as_bytes();
            if end < name.len() + 1 {
                return Err(Error::Size);
            }
            end -= name.len();
            bytes[end..end + name.len()].copy_from_slice(name);
            end -= 1;
            bytes[end] = b'/';
            current = m.parent;
        }
        Err(Error::Corrupt)
    }
}
