// SPDX-License-Identifier: Apache-2.0
use rustic_abi::ipc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    Quota,
    Ipc(ipc::Error),
    Protocol,
}
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
pub(crate) fn decode(value: u64) -> Result<u64, Error> {
    use rustic_abi::process;
    if value == process::NOT_SUPPORTED {
        return Err(Error::Unsupported);
    }
    if value == process::QUOTA {
        return Err(Error::Quota);
    }
    for error in [
        ipc::Error::Handle,
        ipc::Error::Denied,
        ipc::Error::Address,
        ipc::Error::Size,
        ipc::Error::Version,
        ipc::Error::Message,
        ipc::Error::WouldBlock,
        ipc::Error::Closed,
        ipc::Error::Cancelled,
        ipc::Error::Quota,
    ] {
        if value == error.code() {
            return Err(Error::Ipc(error));
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_distinct_errors_and_success_values() {
        assert_eq!(decode(0), Ok(0));
        assert_eq!(decode(88), Ok(88));
        assert_eq!(decode(u64::MAX), Err(Error::Unsupported));
        assert_eq!(decode(u64::MAX - 1), Err(Error::Quota));
        assert_eq!(
            decode(ipc::Error::Cancelled.code()),
            Err(Error::Ipc(ipc::Error::Cancelled))
        );
        assert_eq!(
            decode(ipc::Error::Quota.code()),
            Err(Error::Ipc(ipc::Error::Quota))
        );
    }
}
