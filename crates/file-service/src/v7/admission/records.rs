// SPDX-License-Identifier: Apache-2.0
//! Retained V7 admission records as the caller's subject and scope may see
//! them, and their projection into the admission status and observation wire
//! types.
//!
//! Only records with an admission number are admissions; a direct tracked
//! write under the same retry identity is not one and is answered as missing.
//! As for receipt lookups, a record outside the grant scope is
//! indistinguishable from a missing one.
use super::super::{Grant7, scope};
use crate::reply;
use rustic_abi::files::{
    Error,
    admission::{AdmissionId, ObservationV2, State, Status},
    operation::{Instance, Retry},
    reference::Workspace,
};
use rustic_fs::{
    Volume7, WriteIdentity7,
    format7::{Record7, RecordState},
};

/// The subject's admission named by `id`.
pub(super) fn by_id(volume: &Volume7, grant: Grant7, id: AdmissionId) -> Result<Record7, Error> {
    let header = volume.header().map_err(reply::error)?;
    if id.lineage() != header.lineage {
        return Err(Error::Lineage);
    }
    volume
        .retained_records()
        .map_err(reply::error)?
        .iter()
        .flatten()
        .find(|record| {
            record.subject == grant.subject
                && record.admission_number != 0
                && record.admission_number == id.number()
        })
        .filter(|record| visible(volume, grant, record))
        .copied()
        .ok_or(Error::OutcomeUnknown)
}

/// The subject's admission under `workspace` and `retry`. Missing and hidden
/// are `OutcomeUnknown` in the current epoch and `ExpiredEpoch` otherwise.
pub(super) fn by_retry(
    volume: &Volume7,
    grant: Grant7,
    workspace: Workspace,
    retry: Retry,
) -> Result<Record7, Error> {
    let header = volume.header().map_err(reply::error)?;
    if workspace.lineage() != header.lineage {
        return Err(Error::Lineage);
    }
    let epoch = retry.epoch.value();
    let hidden = if epoch == header.epoch {
        Error::OutcomeUnknown
    } else {
        Error::ExpiredEpoch
    };
    volume
        .retained_records()
        .map_err(reply::error)?
        .iter()
        .flatten()
        .find(|record| {
            record.subject == grant.subject
                && record.admission_number != 0
                && record.workspace == workspace.root()
                && record.retry_epoch == epoch
                && record.retry_key == retry.key.value()
        })
        .filter(|record| visible(volume, grant, record))
        .copied()
        .ok_or(hidden)
}

/// The volume identity that execution or cancellation of `record` names.
pub(super) fn identity(record: &Record7) -> WriteIdentity7 {
    WriteIdentity7 {
        subject: record.subject,
        workspace: record.workspace,
        object: record.object,
        instance: record.instance,
        retry_epoch: record.retry_epoch,
        retry_key: record.retry_key,
    }
}

/// Minimal immutable status of an admission record.
pub(super) fn status(lineage: [u8; 16], record: &Record7) -> Result<Status, Error> {
    Ok(Status {
        id: AdmissionId::new(lineage, record.admission_number)?,
        state: match record.state {
            RecordState::Admitted => State::Admitted,
            RecordState::AdmittedCommitted => State::Committed,
            RecordState::Cancelled => State::Cancelled,
            // Never an admission; callers filter these out first.
            RecordState::DirectCommitted => return Err(Error::Corrupt),
        },
        service_instance: Instance::new(lineage, record.instance)?,
        terminal: record.terminal,
    })
}

/// Retained observation: V7 has no live execution, so every admission is
/// observed as its durable record, with the retained cause when cancelled.
pub(super) fn observation(lineage: [u8; 16], record: &Record7) -> Result<ObservationV2, Error> {
    Ok(ObservationV2::Retained {
        status: status(lineage, record)?,
        prevention: record.prevention.map(crate::admission::prevention_reason),
    })
}

fn visible(volume: &Volume7, grant: Grant7, record: &Record7) -> bool {
    scope::retained_visible(volume, grant.scope, record.workspace, record.object)
}
