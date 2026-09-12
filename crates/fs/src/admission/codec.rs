// SPDX-License-Identifier: Apache-2.0
use super::{AdmissionState, PreventionReason, Stored};
use crate::{Error, Receipt};

impl Stored {
    pub(crate) fn encode(self, p: &mut [u8]) {
        p[64..72].copy_from_slice(&self.number.to_le_bytes());
        p[72..80].copy_from_slice(&self.terminal.to_le_bytes());
        p[80] = match self.state {
            AdmissionState::Admitted => 1,
            AdmissionState::Cancelled => 2,
            AdmissionState::Committed => 3,
        };
        p[81] = self.prevention.map_or(0, |reason| reason as u8);
    }

    pub(crate) fn decode(
        p: &[u8],
        version: u8,
        receipt: Receipt,
        namespace: Option<(u32, u64)>,
        sequence: u64,
    ) -> Result<Option<Self>, Error> {
        if p[64..512].iter().all(|v| *v == 0) {
            return Ok(None);
        }
        if version < 4 || (version == 4 && p[81] != 0) || p[82..512].iter().any(|v| *v != 0) {
            return Err(Error::Corrupt);
        }
        let number = u64::from_le_bytes(p[64..72].try_into().unwrap());
        let terminal = u64::from_le_bytes(p[72..80].try_into().unwrap());
        let state = match p[80] {
            1 => AdmissionState::Admitted,
            2 => AdmissionState::Cancelled,
            3 => AdmissionState::Committed,
            _ => return Err(Error::Corrupt),
        };
        let prevention = if state == AdmissionState::Cancelled {
            Some(match p[81] {
                0 => PreventionReason::Unknown,
                1 => PreventionReason::Requested,
                2 => PreventionReason::VersionConflict,
                3 => PreventionReason::AuthorityLost,
                _ => return Err(Error::Corrupt),
            })
        } else if p[81] == 0 {
            None
        } else {
            return Err(Error::Corrupt);
        };
        if number == 0
            || number > sequence
            || receipt.previous >= number
            || namespace.is_none_or(|(_, i)| i == 0 || i > number)
        {
            return Err(Error::Corrupt);
        }
        match state {
            AdmissionState::Admitted if terminal != 0 || receipt.committed != 0 => {
                return Err(Error::Corrupt);
            }
            AdmissionState::Cancelled
                if terminal <= number || terminal > sequence || receipt.committed != 0 =>
            {
                return Err(Error::Corrupt);
            }
            AdmissionState::Committed
                if terminal <= number || terminal > sequence || receipt.committed != terminal =>
            {
                return Err(Error::Corrupt);
            }
            _ => (),
        }
        Ok(Some(Self {
            number,
            state,
            terminal,
            prevention,
        }))
    }
}
