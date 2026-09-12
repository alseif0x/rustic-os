// SPDX-License-Identifier: Apache-2.0
//! Bounded contract selections owned by one client, never transferable authority.
use super::Client;
use rustic_abi::{
    files::{
        Error,
        admission::AdmissionId,
        lifecycle::{CancelAck, Operation},
        negotiation::Descriptor,
    },
    services::{Availability, Method},
};

#[derive(Default)]
pub(super) struct Selection {
    context: u32,
    methods: [Option<Descriptor>; 2],
}

fn slot(method: Method) -> Result<usize, Error> {
    match method {
        Method::OperationsGet => Ok(0),
        Method::OperationsCancel => Ok(1),
        _ => Err(Error::Unsupported),
    }
}

impl<P: crate::rpc::Progress> Client<P> {
    /// Select a reviewed lifecycle method on this connection without borrowing
    /// the client for the lifetime of the selection. At most get and cancel are
    /// retained. The returned descriptor is metadata, not an executable token.
    ///
    /// A failed refresh clears both selections, except an authenticated Busy
    /// reply on the same context preserves previous support for live control.
    /// Busy never adds a selection. Rebind always clears both selections.
    /// A context change observed here or by a selected call clears old entries.
    /// Selection is explicit and may return Busy during a publication. Select
    /// before scheduling: later calls do not renegotiate, downgrade or retry.
    pub fn select_lifecycle(&mut self, method: Method) -> Result<Descriptor, Error> {
        let mut previous = core::mem::take(&mut self.selection);
        let index = slot(method)?;
        if previous.context != self.context {
            previous = Selection::default();
        }
        let descriptor = match self.lifecycle_descriptor(method) {
            Ok(descriptor) => descriptor,
            Err(Error::Busy) => {
                // Publication temporarily defers discovery, not existing live
                // control. The transport already validated sender and context.
                self.selection = previous;
                return Err(Error::Busy);
            }
            Err(error) => return Err(error),
        };
        previous.context = self.context;
        previous.methods[index] = Some(descriptor);
        self.selection = previous;
        Ok(descriptor)
    }

    /// Inspect through the selected profile. The service checks current rights,
    /// subject and scope on every call; selection never caches permission.
    pub fn inspect_selected(&mut self, id: AdmissionId) -> Result<Operation, Error> {
        self.require_selection(Method::OperationsGet)?;
        self.operation_inspect(id)
    }

    /// Send one cancellation through the selected profile. An uncertain reply
    /// stays uncertain and is never automatically retried or followed by a read.
    pub fn cancel_selected(&mut self, id: AdmissionId) -> Result<CancelAck, Error> {
        self.require_selection(Method::OperationsCancel)?;
        self.operation_cancel(id)
    }

    fn require_selection(&mut self, method: Method) -> Result<(), Error> {
        if self.selection.context != self.context {
            self.selection = Selection::default();
        }
        match self.selection.methods[slot(method)?] {
            Some(descriptor) if descriptor.availability == Availability::Available => Ok(()),
            _ => Err(Error::Unavailable),
        }
    }
}
