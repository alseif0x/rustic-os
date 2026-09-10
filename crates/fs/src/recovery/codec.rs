// SPDX-License-Identifier: Apache-2.0
use super::{RECOVERY_SECTORS, Receipt, Record, Recovery, Retry};
use crate::{Error, MAX_FILE};
impl Recovery {
    pub(crate) fn encode(&self) -> [u8; RECOVERY_SECTORS * 512] {
        let mut b = [0; RECOVERY_SECTORS * 512];
        b[..8].copy_from_slice(if self.scoped {
            b"RUSTREC2"
        } else {
            b"RUSTREC1"
        });
        b[8..24].copy_from_slice(&self.lineage);
        b[24..32].copy_from_slice(&self.epoch.to_le_bytes());
        for (slot, record) in self.records.iter().enumerate() {
            let Some(r) = record else { continue };
            let p = &mut b[512 + slot * 1536..512 + (slot + 1) * 1536];
            p[..8].copy_from_slice(&r.subject.to_le_bytes());
            p[8..16].copy_from_slice(&r.receipt.retry.epoch.to_le_bytes());
            p[16..24].copy_from_slice(&r.receipt.retry.key.to_le_bytes());
            p[24..28].copy_from_slice(&r.receipt.id.to_le_bytes());
            p[28..30].copy_from_slice(&r.receipt.length.to_le_bytes());
            p[32..40].copy_from_slice(&r.receipt.previous.to_le_bytes());
            p[40..48].copy_from_slice(&r.receipt.committed.to_le_bytes());
            if let Some((workspace, instance)) = r.namespace {
                p[48..52].copy_from_slice(&workspace.to_le_bytes());
                p[56..64].copy_from_slice(&instance.to_le_bytes());
            }
            p[512..].copy_from_slice(&r.bytes);
        }
        b
    }
    pub(crate) fn decode(
        b: &[u8; RECOVERY_SECTORS * 512],
        sequence: u64,
        next: u32,
        scoped: bool,
    ) -> Result<Self, Error> {
        if &b[..8] != (if scoped { b"RUSTREC2" } else { b"RUSTREC1" })
            || b[32..512].iter().any(|v| *v != 0)
        {
            return Err(Error::Corrupt);
        }
        let mut value = Self::new(b[8..24].try_into().unwrap()).map_err(|_| Error::Corrupt)?;
        value.scoped = scoped;
        value.epoch = u64::from_le_bytes(b[24..32].try_into().unwrap());
        if value.epoch == 0 || value.epoch > sequence {
            return Err(Error::Corrupt);
        }
        for slot in 0..value.records.len() {
            let p = &b[512 + slot * 1536..512 + (slot + 1) * 1536];
            if p.iter().all(|v| *v == 0) {
                continue;
            }
            let workspace = u32::from_le_bytes(p[48..52].try_into().unwrap());
            let instance = u64::from_le_bytes(p[56..64].try_into().unwrap());
            let namespace = if workspace == 0 && instance == 0 {
                None
            } else {
                Some((workspace, instance))
            };
            let receipt = Receipt {
                retry: Retry {
                    lineage: value.lineage,
                    epoch: u64::from_le_bytes(p[8..16].try_into().unwrap()),
                    key: u64::from_le_bytes(p[16..24].try_into().unwrap()),
                },
                id: u32::from_le_bytes(p[24..28].try_into().unwrap()),
                length: u16::from_le_bytes(p[28..30].try_into().unwrap()),
                previous: u64::from_le_bytes(p[32..40].try_into().unwrap()),
                committed: u64::from_le_bytes(p[40..48].try_into().unwrap()),
            };
            let subject = u64::from_le_bytes(p[..8].try_into().unwrap());
            if subject == 0
                || receipt.retry.epoch != value.epoch
                || receipt.retry.key == 0
                || receipt.id <= 4
                || receipt.id >= next
                || receipt.previous == 0
                || receipt.previous >= receipt.committed
                || receipt.committed > sequence
                || receipt.length as usize > MAX_FILE
                || p[30..32] != [0; 2]
                || p[52..56].iter().chain(&p[64..512]).any(|v| *v != 0)
                || namespace.is_some_and(|(w, i)| {
                    !scoped
                        || w == 0
                        || w >= next
                        || w == receipt.id
                        || i == 0
                        || i > receipt.committed
                })
                || p[512 + receipt.length as usize..].iter().any(|v| *v != 0)
                || value.records.iter().flatten().any(|r| {
                    r.receipt.committed == receipt.committed
                        || r.subject == subject
                            && r.namespace.map(|n| n.0) == namespace.map(|n| n.0)
                            && r.receipt.retry == receipt.retry
                })
            {
                return Err(Error::Corrupt);
            }
            value.records[slot] = Some(Record {
                subject,
                receipt,
                namespace,
                bytes: p[512..].try_into().unwrap(),
            });
        }
        Ok(value)
    }
}
