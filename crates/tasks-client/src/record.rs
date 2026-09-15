// SPDX-License-Identifier: Apache-2.0
//! Where one client keeps its single recovery record, and how it reaches it.
//!
//! The location is a pure value: resolving it to an object needs file authority,
//! but the rules about which locations are legal do not, so they live here.
/// Resolving a location is only reachable where a file client exists, so the
/// rules compile for the guest and for their host tests.
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
use crate::Error;

/// Where one client keeps its single recovery record.
///
/// Each semantic client owns a distinct record: two clients sharing one record
/// would each treat the other's unresolved intent as their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record<'a> {
    /// A record the client names itself, below the volume root.
    Path { directory: &'a str, name: &'a str },
    /// A record the client only knows as an object it was granted.
    Object(u32),
}

/// How the client reaches its record once the location is known to be legal.
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place<'a> {
    /// Look the name up below the volume root; create it on first use.
    Named { directory: &'a str, name: &'a str },
    /// Stat this granted object; it must already exist and is never created.
    Granted(u32),
    /// The location cannot name a record at all.
    Invalid,
}

impl<'a> Record<'a> {
    /// `directory` is resolved from the volume root, not from a working
    /// directory, so the record cannot move with the caller's navigation.
    pub const fn path(directory: &'a str, name: &'a str) -> Self {
        Self::Path { directory, name }
    }

    /// A record the client holds only as an object id, as a launcher grants it.
    /// The client never resolves a path for it and never creates it.
    pub const fn object(id: u32) -> Self {
        Self::Object(id)
    }
}

#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
impl<'a> Record<'a> {
    pub(crate) const fn place(&self) -> Place<'a> {
        match self {
            Self::Path { directory, name } => Place::Named { directory, name },
            // Object zero is the volume root, which is a directory and never a
            // record, so it is refused before any file exchange.
            Self::Object(0) => Place::Invalid,
            Self::Object(id) => Place::Granted(*id),
        }
    }

    /// The location to create when the record is absent. An object record has
    /// none: the client was granted an existing object, not a name.
    pub(crate) const fn creation(&self) -> Result<(&'a str, &'a str), Error> {
        match self.place() {
            Place::Named { directory, name } => Ok((directory, name)),
            Place::Granted(_) | Place::Invalid => Err(Error::Journal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_named_record_is_created_by_the_client() {
        let named = Record::path("/config", "tasks-intent");
        assert_eq!(
            named.place(),
            Place::Named {
                directory: "/config",
                name: "tasks-intent"
            }
        );
        assert_eq!(named.creation(), Ok(("/config", "tasks-intent")));
        // A granted object is reached by id and never created, so there is no
        // location to create it at.
        assert_eq!(Record::object(9).place(), Place::Granted(9));
        assert_eq!(Record::object(9).creation(), Err(Error::Journal));
        // The volume root is not a record, and neither is an unset grant.
        assert_eq!(Record::object(0).place(), Place::Invalid);
        assert_eq!(Record::object(0).creation(), Err(Error::Journal));
    }
}
