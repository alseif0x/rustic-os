// SPDX-License-Identifier: Apache-2.0
//! Canonical bounded ASCII encoding; parsing never indexes unvalidated UTF-8.
use super::{Epoch, Error, Resource, Version, Workspace};
use core::{fmt, str::FromStr};

fn hex<const N: usize>(bytes: &[u8]) -> Result<[u8; N], Error> {
    if bytes.len() != N * 2 {
        return Err(Error::Invalid);
    }
    let mut result = [0; N];
    for (out, pair) in result.iter_mut().zip(bytes.as_chunks::<2>().0) {
        let digit = |b| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(Error::Invalid),
        };
        *out = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Ok(result)
}
fn lineage(f: &mut fmt::Formatter<'_>, value: [u8; 16]) -> fmt::Result {
    for byte in value {
        write!(f, "{byte:02x}")?;
    }
    Ok(())
}
impl fmt::Display for Workspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ws_")?;
        lineage(f, self.lineage)?;
        write!(f, "_{:08x}", self.root)
    }
}
impl FromStr for Workspace {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let b = value.as_bytes();
        if b.len() != 44 || &b[..3] != b"ws_" || b[35] != b'_' {
            return Err(Error::Invalid);
        }
        Self::new(hex(&b[3..35])?, u32::from_be_bytes(hex(&b[36..44])?))
    }
}
impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("rs_")?;
        lineage(f, self.workspace.lineage)?;
        write!(f, "_{:08x}_{:08x}", self.workspace.root, self.object)
    }
}
impl FromStr for Resource {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let b = value.as_bytes();
        if b.len() != 53 || &b[..3] != b"rs_" || b[35] != b'_' || b[44] != b'_' {
            return Err(Error::Invalid);
        }
        let workspace = Workspace::new(hex(&b[3..35])?, u32::from_be_bytes(hex(&b[36..44])?))?;
        Self::new(workspace, u32::from_be_bytes(hex(&b[45..53])?))
    }
}
impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v_{:016x}", self.0)
    }
}
impl FromStr for Version {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let b = value.as_bytes();
        if b.len() != 18 || &b[..2] != b"v_" {
            return Err(Error::Invalid);
        }
        Self::new(u64::from_be_bytes(hex(&b[2..])?))
    }
}
impl fmt::Display for Epoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e_{:016x}", self.0)
    }
}
impl FromStr for Epoch {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let b = value.as_bytes();
        if b.len() != 18 || &b[..2] != b"e_" {
            return Err(Error::Invalid);
        }
        Self::new(u64::from_be_bytes(hex(&b[2..])?))
    }
}
