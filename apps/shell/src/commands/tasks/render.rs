// SPDX-License-Identifier: Apache-2.0
//! Terminal presentation of owner-client results. It makes no decisions.
use super::output;
use rustic_tasks_client::{Applied, Committed, Listing, Note, Recovery, Report};
use rustic_tasks_contract::State;

/// Prints notes while an operation is still running, so what is already durable
/// is reported even when the operation then fails.
pub(super) struct Console {
    verb: &'static str,
}

impl Console {
    pub(super) fn applied() -> Self {
        Self { verb: "applied" }
    }
    pub(super) fn recovered() -> Self {
        Self { verb: "recovered" }
    }
}

impl Report for Console {
    fn note(&mut self, note: Note) {
        match note {
            Note::Retained { journal } => output::format(format_args!(
                "task intent={journal} retained before submission\r\n"
            )),
            Note::Ambiguous => output::text(
                "Intent retained; use tasks recover (restart files first if disconnected). No automatic retry.\r\n",
            ),
            Note::Unresolved { journal } => output::format(format_args!(
                "intent={journal} unresolved; target not resubmitted\r\n"
            )),
            Note::Unverified { journal, version } => output::format(format_args!(
                "intent={journal} committed at version={version}; current bytes could not be verified; intent retained\r\n"
            )),
            Note::CleanupIncomplete(committed) => {
                effect(self.verb, committed);
                output::text(
                    "Effect verified; intent cleanup incomplete. Use tasks recover before another edit.\r\n",
                );
            }
            #[cfg(feature = "tasks-acceptance")]
            Note::ReplyDiscarded => {
                output::text("Task acceptance: real commit reply discarded without decoding\r\n")
            }
        }
    }
}

fn effect(verb: &str, committed: Committed) {
    output::format(format_args!(
        "task={} {} version={} intent={} bytes verified\r\n",
        committed.task_id, verb, committed.version, committed.journal
    ));
}

pub(super) fn applied(applied: Applied) {
    match applied {
        Applied::Unchanged { task_id, version } => {
            output::format(format_args!(
                "task={task_id} unchanged version={version}\r\n"
            ));
        }
        Applied::Committed(committed) => effect("applied", committed),
    }
}

pub(super) fn recovery(recovery: Recovery) {
    match recovery {
        Recovery::Absent => output::text(
            "No retained task intent. This does not prove a forgotten effect never happened.\r\n",
        ),
        Recovery::Recovered(committed) => effect("recovered", committed),
    }
}

pub(super) fn listing(listing: &Listing) {
    for row in listing.rows() {
        let state = match row.state {
            State::Open => "open",
            State::Done => "done",
        };
        output::format(format_args!("{} [{}] ", row.id, state));
        output::bytes(&row.title[..usize::from(row.title_len)]);
        output::text("\r\n");
    }
    output::format(format_args!("{} tasks\r\n", listing.count()));
    if let Some(summary) = listing.summary() {
        output::format(format_args!(
            "preview task={} changed={} source_version={}; not applied\r\n",
            summary.task_id,
            u8::from(summary.changed),
            summary.version
        ));
    }
}
