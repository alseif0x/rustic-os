// SPDX-License-Identifier: Apache-2.0
//! Owner-client failure vocabulary. The embedding application owns the wording.
use rustic_sdk::abi::files::Error as File;

/// Why an owner-client operation did not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A file-service refusal, forwarded unchanged.
    File(File),
    /// A supervisor or native application refusal code.
    Service(u64),
    /// The document is not the one the edit claims: the native application
    /// rejected it, or a candidate did not parse, match its summary or show the
    /// effect of its command.
    Document,
    /// The native application reported capacity exhaustion.
    Capacity,
    /// Replacement is unavailable until the owner explicitly enables writes.
    Enable,
    /// The recovery record is missing, corrupt, or not the object it claims.
    Journal,
    /// One unresolved intent with this journal version blocks mutations.
    Pending(u64),
}

impl Error {
    /// Whether a file refusal proves a synchronous submission did not publish.
    ///
    /// Only these six canonical server refusals are conclusive. Transport loss,
    /// interruption and process death stay ambiguous, so the caller must retain
    /// the intent instead of clearing it.
    pub fn conclusive(&self) -> bool {
        matches!(
            self,
            Self::File(
                File::Full
                    | File::Version
                    | File::Denied
                    | File::Revoked
                    | File::Expired
                    | File::ReadOnly
            )
        )
    }
}

impl From<File> for Error {
    fn from(error: File) -> Self {
        Self::File(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_canonical_refusals_prove_no_publication() {
        for error in [
            File::Full,
            File::Version,
            File::Denied,
            File::Revoked,
            File::Expired,
            File::ReadOnly,
        ] {
            assert!(Error::File(error).conclusive());
        }
        // Ambiguity must never clear a retained intent: an unknown or lost
        // outcome does not prove the effect is absent.
        for error in [
            File::Uncertain,
            File::OutcomeUnknown,
            File::Interrupted,
            File::Protocol,
            File::Io,
            File::Unavailable,
            File::Closed,
            File::Busy,
            File::NotFound,
            File::ExpiredEpoch,
            File::IdempotencyConflict,
        ] {
            assert!(!Error::File(error).conclusive());
        }
        assert!(!Error::Service(4).conclusive());
        assert!(!Error::Pending(7).conclusive());
    }
}
