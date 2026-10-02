// SPDX-License-Identifier: Apache-2.0
//! Volatile execution view retained while V7 executes or prevents one admission.
use super::scope::Scope7;
use rustic_abi::files::admission::{Activity, ActivityPhase};
use rustic_fs::Publication7Phase;

pub(in crate::v7) struct Active7 {
    activity: Activity,
    pub(super) scope: Scope7,
}

impl Active7 {
    pub(in crate::v7) fn new(scope: Scope7) -> Self {
        Self {
            activity: Activity {
                id: scope.id,
                service_instance: scope.instance,
                phase: ActivityPhase::Running,
                cancel_requested: false,
                io_pending: false,
            },
            scope,
        }
    }

    pub(super) fn observation(&self) -> Activity {
        self.activity
    }

    pub(in crate::v7) fn stopping(&self) -> bool {
        self.activity.cancel_requested
    }

    pub(in crate::v7) fn stop(&mut self) {
        self.activity.cancel_requested = true;
        if self.activity.phase == ActivityPhase::Running {
            self.activity.phase = ActivityPhase::Stopping;
        }
    }

    pub(in crate::v7) fn observe(
        &mut self,
        phase: Publication7Phase,
        pending: bool,
        cleanup: bool,
    ) {
        self.activity.io_pending = pending;
        self.activity.phase = if cleanup {
            ActivityPhase::Stopping
        } else if matches!(
            phase,
            Publication7Phase::Settling | Publication7Phase::Committed
        ) {
            ActivityPhase::Settling
        } else if self.stopping() {
            ActivityPhase::Stopping
        } else {
            ActivityPhase::Running
        };
    }

    pub(super) fn packet(
        &self,
        op: u8,
        context: u32,
    ) -> Result<rustic_abi::files::Packet, rustic_abi::files::Error> {
        self.activity.packet(op, context)
    }
}
