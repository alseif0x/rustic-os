// SPDX-License-Identifier: Apache-2.0
//! Snapshot the verified scope and endpoint bindings for one retained V7 admission.
use super::super::{
    Grant7,
    grants::{CLIENTS7, Grants},
    scope,
};
use crate::reply;
use rustic_abi::files::{
    CANCEL_RIGHT, Error, INSPECT_RIGHT, WRITE_RIGHT,
    admission::{AdmissionId, State, Status},
    operation::Instance,
};
use rustic_fs::{
    Volume7,
    format7::{Record7, RecordState},
};

#[derive(Clone, Copy)]
struct Binding7 {
    grant: Grant7,
    allowed: u8,
}

/// A compact record identity/status plus the live grants that could reach it
/// when the storage snapshot was made. Requests must still match the exact
/// saved grant, including its optional second file scope.
#[derive(Clone, Copy)]
pub(in crate::v7) struct Scope7 {
    pub(super) id: AdmissionId,
    pub(super) instance: Instance,
    pub(super) status: Status,
    pub(super) subject: u64,
    bindings: [Option<Binding7>; CLIENTS7],
}

impl Scope7 {
    pub(in crate::v7) fn capture(
        volume: &Volume7,
        grants: &Grants,
        record: Record7,
    ) -> Result<Self, Error> {
        let lineage = volume.header().map_err(reply::error)?.lineage;
        let status = super::records::status(lineage, &record)?;
        let mut bindings = [None; CLIENTS7];
        for (slot, binding) in bindings.iter_mut().enumerate() {
            let Some(grant) = grants.grant_at(slot) else {
                continue;
            };
            let mut allowed = 0;
            if grant.subject == record.subject
                && scope::retained_visible(volume, grant, record.workspace, record.object)
            {
                // Inspection and cancellation disclose only the retained
                // admission identity/status. They do not imply WRITE.
                allowed |= INSPECT_RIGHT | CANCEL_RIGHT;
                if scope::authorized_resource(volume, grant, record.workspace, record.object)
                    .is_ok()
                {
                    allowed |= WRITE_RIGHT;
                }
            }
            *binding = Some(Binding7 { grant, allowed });
        }
        Ok(Self {
            id: status.id,
            instance: status.service_instance,
            status,
            subject: record.subject,
            bindings,
        })
    }

    /// Check live peer, context, deadline and requested rights, then require
    /// the same scope/second-scope binding that supplied this proof.
    pub(super) fn check(
        &self,
        grants: &Grants,
        slot: usize,
        peer: u64,
        context: u32,
        now: u64,
        right: u8,
    ) -> Result<Grant7, Error> {
        let grant = grants.check(slot, peer, context, now)?;
        grant.holds(right)?;
        let saved = self
            .bindings
            .get(slot)
            .copied()
            .flatten()
            .ok_or(Error::OutcomeUnknown)?;
        if saved.grant != grant || saved.allowed & right != right {
            return Err(Error::OutcomeUnknown);
        }
        Ok(grant)
    }

    pub(super) fn terminal_state(&self) -> bool {
        self.status.state != State::Admitted
    }
}

pub(super) fn is_admission(record: &Record7) -> bool {
    record.admission_number != 0 && record.state != RecordState::DirectCommitted
}
