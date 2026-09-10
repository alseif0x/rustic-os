// SPDX-License-Identifier: Apache-2.0
//! Authenticated evidence visibility and tracked staging. The volume owns durability.
use crate::{Grant, Server, reply};
use rustic_abi::files::{
    recovery::{Receipt, Retry},
    *,
};
use rustic_fs::Disk;
fn native(r: rustic_fs::Receipt) -> Receipt {
    Receipt {
        retry: Retry {
            lineage: r.retry.lineage,
            epoch: r.retry.epoch,
            key: r.retry.key,
        },
        id: r.id,
        previous: r.previous,
        committed: r.committed,
        length: r.length,
    }
}
fn stored(r: Retry) -> rustic_fs::Retry {
    rustic_fs::Retry {
        lineage: r.lineage,
        epoch: r.epoch,
        key: r.key,
    }
}
impl Server {
    pub(super) fn recovery_request(
        &mut self,
        disk: &mut impl Disk,
        slot: usize,
        grant: Grant,
        p: Packet,
    ) -> Result<Packet, Error> {
        grant.inspect(&self.volume, p.id)?;
        let mut r = Packet::new(p.op);
        r.context = p.context;
        match p.op {
            RECOVERY => {
                let (lineage, epoch) = self.volume.recovery_info().map_err(reply::error)?;
                r.data[..16].copy_from_slice(&lineage);
                r.data[16..24].copy_from_slice(&epoch.to_le_bytes());
                r.count = 24;
                r.arg = rustic_fs::RETAINED as u32;
            }
            RECEIPT => {
                let receipt = self
                    .volume
                    .receipt(grant.subject, stored(Retry::decode(p.payload())?))
                    .map_err(reply::error)?;
                if receipt.id != p.id {
                    return Err(Error::OutcomeUnknown);
                }
                r = native(receipt).packet(r);
            }
            TRACK_BEGIN => {
                let retry = stored(Retry::decode(p.payload())?);
                match self.volume.receipt(grant.subject, retry) {
                    Ok(receipt) => grant.inspect(&self.volume, receipt.id)?, // Authorize the retained target too.
                    Err(rustic_fs::Error::OutcomeUnknown) => {
                        grant.access(&self.volume, p.id, true)?
                    }
                    Err(e) => return Err(reply::error(e)),
                }
                self.transfers.begin(slot, &p)?;
            }
            COMMIT => {
                let transfer = self.transfers.take(slot, &p)?;
                let retry = stored(transfer.retry.ok_or(Error::Protocol)?);
                // A replay under inspection-only authority cannot admit a fresh write.
                match self.volume.receipt(grant.subject, retry) {
                    Ok(receipt) => grant.inspect(&self.volume, receipt.id)?,
                    Err(rustic_fs::Error::OutcomeUnknown) => {
                        grant.access(&self.volume, p.id, true)?
                    }
                    Err(error) => return Err(reply::error(error)),
                }
                let receipt = self
                    .volume
                    .replace_tracked(
                        disk,
                        grant.subject,
                        retry,
                        transfer.id,
                        transfer.version,
                        &transfer.data[..transfer.total],
                    )
                    .map_err(reply::error)?;
                r = native(receipt).packet(r);
            }
            _ => return Err(Error::Protocol),
        }
        Ok(r)
    }
}
