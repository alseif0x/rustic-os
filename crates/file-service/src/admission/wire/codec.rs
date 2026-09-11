// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{
    Error, Packet,
    admission::{AdmissionId, State, Status},
    operation::{Instance, Replacement},
};

pub(super) fn stored(request: Replacement) -> rustic_fs::Replacement {
    rustic_fs::Replacement {
        workspace: request.workspace.root(),
        id: request.resource.object(),
        version: request.expected_version.value(),
        retry: rustic_fs::Retry {
            lineage: request.workspace.lineage(),
            epoch: request.retry.epoch.value(),
            key: request.retry.key.value(),
        },
    }
}
pub(super) fn id(value: AdmissionId) -> rustic_fs::AdmissionId {
    rustic_fs::AdmissionId {
        lineage: value.lineage(),
        number: value.number(),
    }
}
pub(super) fn status(old: rustic_fs::Admission<'_>, op: u8, context: u32) -> Result<Packet, Error> {
    Status {
        id: AdmissionId::new(old.status.id.lineage, old.status.id.number)?,
        state: match old.status.state {
            rustic_fs::AdmissionState::Admitted => State::Admitted,
            rustic_fs::AdmissionState::Cancelled => State::Cancelled,
            rustic_fs::AdmissionState::Committed => State::Committed,
        },
        service_instance: Instance::new(old.status.id.lineage, old.instance)?,
        terminal: old.status.terminal,
    }
    .packet(op, context)
}
