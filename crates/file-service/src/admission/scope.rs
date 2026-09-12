// SPDX-License-Identifier: Apache-2.0
//! Immutable namespace proof, usable only while the storage namespace is borrowed.
use super::Caller;
use crate::{CLIENTS, Clients, Grant, Server};
use rustic_abi::files::{
    CANCEL_RIGHT, Error, INSPECT_RIGHT, WRITE_RIGHT, admission::AdmissionId, operation::Instance,
};
use rustic_fs::Admission;

#[derive(Clone, Copy)]
struct Binding {
    grant: Grant,
    allowed: u8,
}

#[derive(Clone, Copy)]
pub(super) struct Scope {
    pub(super) id: AdmissionId,
    pub(super) instance: Instance,
    bindings: [Option<Binding>; CLIENTS],
}
impl Scope {
    pub(super) fn new(server: &Server, subject: u64, old: &Admission<'_>) -> Result<Self, Error> {
        let mut bindings = [None; CLIENTS];
        for (slot, binding) in bindings.iter_mut().enumerate() {
            if let Some(grant) = server.grant_at(slot) {
                let mut allowed = 0;
                if grant.subject == subject {
                    for right in [INSPECT_RIGHT, CANCEL_RIGHT] {
                        if grant
                            .operation_scope(
                                &server.volume,
                                old.request.workspace,
                                old.request.id,
                                right,
                            )
                            .is_ok()
                        {
                            allowed |= right;
                        }
                    }
                    if grant.access(&server.volume, old.request.id, true).is_ok() {
                        allowed |= WRITE_RIGHT;
                    }
                }
                *binding = Some(Binding { grant, allowed });
            }
        }
        Ok(Self {
            id: AdmissionId::new(old.status.id.lineage, old.status.id.number)?,
            instance: Instance::new(old.status.id.lineage, old.instance)?,
            bindings,
        })
    }
    /// Recheck the live peer/context/right/deadline every time. No grant issuance
    /// or namespace mutation is permitted while this snapshot supplies authority.
    pub(super) fn check(
        &self,
        clients: &Clients,
        caller: Caller,
        right: u8,
        now: u64,
    ) -> Result<(), Error> {
        let grant = caller.check_right(clients, now, right)?;
        let binding = self
            .bindings
            .get(caller.slot)
            .copied()
            .flatten()
            .ok_or(Error::OutcomeUnknown)?;
        let saved = binding.grant;
        if binding.allowed & right != right
            || grant.peer != saved.peer
            || grant.endpoint != saved.endpoint
            || grant.generation != saved.generation
            || grant.subject != saved.subject
            || grant.scope != saved.scope
        {
            return Err(Error::OutcomeUnknown);
        }
        Ok(())
    }
}
