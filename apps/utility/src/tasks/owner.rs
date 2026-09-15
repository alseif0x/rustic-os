// SPDX-License-Identifier: Apache-2.0
//! The owner-stepped exchange of the second native tasks client.
//!
//! One message is one step. The client borrows its file authority to the owner
//! client for the duration of a single call and keeps no session of its own, so
//! the two objects it was granted are the whole of its authority: the target
//! document it was launched for, and the journal object it records intents in.
//! It resolves no path, creates no record and addresses no supervisor.
use super::{
    report::{self, Trace},
    state::{Collection, Fault},
};
use rustic_sdk::{
    abi::{runtime as k, supervisor::actor as a},
    files::Client as Files,
    ipc::{Endpoint, Message},
    rpc::Blocking,
};
use rustic_tasks_client::{Authority, Client, Cut, Record};

/// The file authority this application lends for one owner client call.
struct Lent<'a> {
    files: &'a mut Files,
}

impl Authority for Lent<'_> {
    type Progress = Blocking;
    fn files(&mut self) -> &mut Files {
        self.files
    }
}

/// Serves the tasks-owner protocol until the owner's channel ends.
///
/// The return values are those of the other persistent loops: the owner is the
/// only accepted sender, and a malformed or undeliverable exchange ends the
/// process instead of being answered as if it were a step.
pub fn run(files: &mut Files, control: &Endpoint, owner: u64, scope: u32, journal: u32) -> u64 {
    let client = Client::new(Record::object(journal));
    let mut state = Collection::new();
    loop {
        if control.wait().is_err() {
            return 1;
        }
        let Ok(message) = control.receive() else {
            return 2;
        };
        if message.sender() != owner {
            return 3;
        }
        let Ok(words) = k::decode(message.payload()) else {
            return 4;
        };
        let reply = step(&client, &mut state, files, scope, words);
        if control
            .send(&Message::new(message.correlation(), &k::encode(reply)).unwrap())
            .is_err()
        {
            return 5;
        }
    }
}

fn step(
    client: &Client<'_>,
    state: &mut Collection,
    files: &mut Files,
    scope: u32,
    w: [u64; 8],
) -> [u64; 8] {
    match w[0] {
        a::TASKS_EDIT => {
            let result = state.edit([w[1], w[2], w[3], w[4], w[5], w[6]]);
            report::step(state, result.err())
        }
        a::TASKS_BEGIN => {
            let result = state.begin(w[1], [w[2], w[3], w[4], w[5]]);
            report::step(state, result.err())
        }
        a::TASKS_CHUNK => {
            let result = state.chunk(w[1], [w[2], w[3], w[4], w[5]]);
            report::step(state, result.err())
        }
        a::TASKS_APPLY => apply(client, state, files, scope, w[1]),
        a::TASKS_STATUS => {
            let pending = client.pending(&mut Lent { files }).map_err(Fault::Client);
            report::status(state, pending)
        }
        a::TASKS_RECOVER => {
            let mut trace = Trace::default();
            let result = client
                .recover(&mut Lent { files }, &mut trace)
                .map_err(Fault::Client);
            report::recovery(result, &trace)
        }
        a::TASKS_FORGET => {
            let result = client
                .forget(&mut Lent { files }, w[1])
                .map_err(Fault::Client);
            report::step(state, result.err())
        }
        _ => report::step(state, Some(Fault::Sequence)),
    }
}

fn apply(
    client: &Client<'_>,
    state: &mut Collection,
    files: &mut Files,
    scope: u32,
    selector: u64,
) -> [u64; 8] {
    let mut trace = Trace::default();
    // The cut is decided before anything is read, so a selector this build does
    // not implement refuses the step without touching a file.
    let Ok(cut) = cut(selector) else {
        return report::applied(Err(Fault::Sequence), &trace);
    };
    let Some(candidate) = state.candidate() else {
        return report::applied(Err(Fault::Sequence), &trace);
    };
    let result = client
        .apply_candidate_cut(&mut Lent { files }, &mut trace, scope, candidate, cut)
        .map_err(Fault::Client);
    // A refusal that proves this submission did not publish concludes the
    // attempt; anything else leaves the plan available to the recovery that
    // must resolve it, so the candidate is kept.
    let concluded = match &result {
        Ok(_) => true,
        Err(Fault::Client(error)) => error.conclusive(),
        Err(Fault::Sequence) => false,
    };
    state.conclude(concluded);
    report::applied(result, &trace)
}

/// The failure cut an apply step selected. Ordinary builds implement `0` only,
/// and refuse every other selector without changing anything.
#[cfg(not(feature = "tasks-acceptance"))]
fn cut(selector: u64) -> Result<Cut, Fault> {
    match selector {
        0 => Ok(Cut::None),
        _ => Err(Fault::Sequence),
    }
}

/// The failure cut an apply step selected, in an explicit acceptance build.
#[cfg(feature = "tasks-acceptance")]
fn cut(selector: u64) -> Result<Cut, Fault> {
    match selector {
        0 => Ok(Cut::None),
        1 => Ok(Cut::Prepared),
        2 => Ok(Cut::LostReply),
        3 => Ok(Cut::LostJournal),
        4 => Ok(Cut::HumanConflict),
        _ => Err(Fault::Sequence),
    }
}
