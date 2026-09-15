// SPDX-License-Identifier: Apache-2.0
//! The owner client: one recovery record plus the operations it protects.
use crate::{
    Applied, Authority, Candidate, Cut, Error, Listing, Pending, Record, Recovery, Relay, Report,
    journal, mutation, transport,
};
use rustic_tasks_contract::preview::Edit;

/// An owner client bound to one recovery record.
///
/// The client holds no session: every operation borrows the caller's authority
/// for the duration of the call, so a caller may rebind or drop its file client
/// between operations without leaving stale state here. Only planning borrows a
/// supervisor relay; everything that owns the record needs file authority alone.
pub struct Client<'a> {
    record: Record<'a>,
}

impl<'a> Client<'a> {
    pub const fn new(record: Record<'a>) -> Self {
        Self { record }
    }

    /// Lists the tasks of one already resolved object.
    pub fn list<L: Relay>(&self, relay: &mut L, scope: u32) -> Result<Listing, Error> {
        self.rows(relay, scope, None)
    }

    /// Calculates an edit without applying it. No request identity is reserved
    /// and no later commit is authorized.
    pub fn preview<L: Relay>(
        &self,
        relay: &mut L,
        scope: u32,
        edit: Edit,
    ) -> Result<Listing, Error> {
        self.rows(relay, scope, Some(edit))
    }

    fn rows<L: Relay>(
        &self,
        relay: &mut L,
        scope: u32,
        edit: Option<Edit>,
    ) -> Result<Listing, Error> {
        Ok(transport::run(relay, scope, edit, false)?.listing)
    }

    /// Plans an edit through the relay, then retains, submits and proves it.
    ///
    /// The path is resolved after the record is known to be idle, so a blocked
    /// intent is reported before any navigation error.
    pub fn apply<L: Relay, R: Report>(
        &self,
        relay: &mut L,
        report: &mut R,
        cwd: u32,
        path: &str,
        edit: Edit,
    ) -> Result<Applied, Error> {
        self.apply_cut(relay, report, cwd, path, edit, Cut::None)
    }

    /// [`Client::apply`] with an explicit failure cut.
    pub fn apply_cut<L: Relay, R: Report>(
        &self,
        relay: &mut L,
        report: &mut R,
        cwd: u32,
        path: &str,
        edit: Edit,
        cut: Cut,
    ) -> Result<Applied, Error> {
        mutation::apply_cut(relay, report, &self.record, cwd, path, edit, cut)
    }

    /// Retains an already planned candidate, submits it once and proves the
    /// outcome, using file authority alone.
    ///
    /// The target is an object the caller already holds; this client resolves no
    /// path and asks no supervisor. The candidate carries the command it
    /// implements, so recovery names the same edit the planner ran.
    pub fn apply_candidate<A: Authority, R: Report>(
        &self,
        authority: &mut A,
        report: &mut R,
        target: u32,
        candidate: &Candidate,
    ) -> Result<Applied, Error> {
        self.apply_candidate_cut(authority, report, target, candidate, Cut::None)
    }

    /// [`Client::apply_candidate`] with an explicit failure cut.
    ///
    /// The cut is taken by the same submission path [`Client::apply_cut`] uses,
    /// so a client that applies a plan it did not make fails where a client that
    /// planned its own edit fails. Ordinary builds have `Cut::None` only.
    pub fn apply_candidate_cut<A: Authority, R: Report>(
        &self,
        authority: &mut A,
        report: &mut R,
        target: u32,
        candidate: &Candidate,
        cut: Cut,
    ) -> Result<Applied, Error> {
        mutation::apply_candidate_cut(authority, report, &self.record, target, candidate, cut)
    }

    /// Queries the recorded retry tuple under the current binding. It never
    /// resubmits, rebuilds or repairs the target.
    pub fn recover<A: Authority, R: Report>(
        &self,
        authority: &mut A,
        report: &mut R,
    ) -> Result<Recovery, Error> {
        mutation::recover(authority, report, &self.record)
    }

    /// Discards the retained record identified by its exact journal version.
    pub fn forget<A: Authority>(&self, authority: &mut A, key: u64) -> Result<(), Error> {
        journal::forget(authority, &self.record, key)
    }

    /// Reports whether an unresolved intent blocks the next mutation. The query
    /// reads the record and changes nothing.
    pub fn pending<A: Authority>(&self, authority: &mut A) -> Result<Pending, Error> {
        journal::pending(authority, &self.record)
    }
}
