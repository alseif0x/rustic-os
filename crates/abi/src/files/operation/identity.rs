// SPDX-License-Identifier: Apache-2.0
use crate::files::{
    Error,
    reference::text::{hex, lineage},
};
use core::{fmt, str::FromStr};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key(u64);
impl Key {
    pub fn new(value: u64) -> Result<Self, Error> {
        if value == 0 {
            Err(Error::Invalid)
        } else {
            Ok(Self(value))
        }
    }
    pub const fn value(self) -> u64 {
        self.0
    }
}
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "k_{:016x}", self.0)
    }
}
impl FromStr for Key {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        let b = s.as_bytes();
        if b.len() != 18 || &b[..2] != b"k_" {
            return Err(Error::Invalid);
        }
        Self::new(u64::from_be_bytes(hex(&b[2..])?))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationId {
    lineage: [u8; 16],
    sequence: u64,
}
impl OperationId {
    pub fn new(lineage: [u8; 16], sequence: u64) -> Result<Self, Error> {
        if lineage == [0; 16] || sequence == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self { lineage, sequence })
    }
    pub const fn lineage(self) -> [u8; 16] {
        self.lineage
    }
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}
impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("op_")?;
        lineage(f, self.lineage)?;
        write!(f, "_{:016x}", self.sequence)
    }
}
impl FromStr for OperationId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        let b = s.as_bytes();
        if b.len() != 52 || &b[..3] != b"op_" || b[35] != b'_' {
            return Err(Error::Invalid);
        }
        Self::new(hex(&b[3..35])?, u64::from_be_bytes(hex(&b[36..])?))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance(OperationId);
impl Instance {
    pub fn new(lineage: [u8; 16], sequence: u64) -> Result<Self, Error> {
        OperationId::new(lineage, sequence).map(Self)
    }
    pub const fn lineage(self) -> [u8; 16] {
        self.0.lineage()
    }
    pub const fn sequence(self) -> u64 {
        self.0.sequence()
    }
}
impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("si_")?;
        lineage(f, self.lineage())?;
        write!(f, "_{:016x}", self.sequence())
    }
}
