// SPDX-License-Identifier: Apache-2.0
//! Release the publication workspace before entering terminal housekeeping.
use crate::admission::control::{Settled, drive_stoppable};
use crate::{ActiveExecution, Caller, Clients, Server, reply};
use rustic_abi::files::{Error, INSPECT_RIGHT};
use rustic_fs::{AdmissionId, PollDisk, Receipt};

impl Server {
    // Publication owns large bounded metadata buffers. Keep its stack frame out
    // of subsequent prevention: these phases never require simultaneous writers.
    #[inline(never)]
    pub(super) fn publish_admission_active(
        &mut self,
        disk: &mut impl PollDisk,
        caller: Caller,
        subject: u64,
        id: AdmissionId,
        active: &mut ActiveExecution,
        control: &mut impl FnMut(&mut Clients, &mut ActiveExecution) -> u64,
    ) -> Result<Settled<Receipt>, Error> {
        let write = self
            .volume
            .prepare_admitted(disk, subject, id)
            .map_err(reply::error)?;
        drive_stoppable(
            &mut self.clients,
            write,
            Some((caller, INSPECT_RIGHT)),
            &mut |clients, phase, pending| {
                active.observe(phase, pending, false);
                let now = control(clients, active);
                (now, active.stopping())
            },
        )
    }
}
