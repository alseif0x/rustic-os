// SPDX-License-Identifier: Apache-2.0
//! The owner-stepped exchange of the second native tasks client.
//!
//! One message is one step. The client borrows its file authority to the owner
//! client for the duration of a single call and keeps no session of its own, so
//! the two objects it was granted are the whole of its authority: the target
//! document it was launched for, and the journal object it records intents in.
//! It resolves no path, creates no record and addresses no supervisor.
//!
//! The candidate bytes are the one thing this child needs memory for, and that
//! memory is dynamic: [`Storage`] reserves a page through the SDK heap when a
//! hand-off announces an edit, the collection assembles the plan directly in the
//! block inside it, and the page is unmapped again as soon as the client holds
//! nothing the bytes are for. A step that arrives while nothing is held is
//! answered without a single mapped page, so the owner can watch this process's
//! heap go from nothing to one page and back over one hand-off and apply.
use super::{
    report::{self, Trace},
    state::{Collection, Fault, Retained},
};
use rustic_sdk::{
    abi::{runtime as k, supervisor::actor as a},
    files::Client as Files,
    ipc::{Endpoint, Message},
    memory::{Block, Heap},
    rpc::Blocking,
};
use rustic_tasks_client::{Authority, Client, Cut, Record};
use rustic_tasks_contract::MAX_BYTES;

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

/// One accepted step: the correlation its answer must carry and its words.
struct Request {
    correlation: u64,
    words: [u64; 8],
}

/// What this child is bound to for its whole life: the record it owns, the file
/// authority it lends one call at a time, the channel it answers on and the two
/// identities it was launched with.
struct Session<'a> {
    client: Client<'a>,
    files: &'a mut Files,
    control: &'a Endpoint,
    owner: u64,
    scope: u32,
}

/// The mapped pages the candidate bytes live in, and the block inside them.
///
/// One page carries the bounded document and the allocator's own bookkeeping,
/// so this heap never grows. It exists only while the client is collecting or
/// holding a plan: `Drop` unmaps the run, which is what returns the process to
/// no mapped pages at all.
struct Storage {
    heap: Heap,
    block: Block,
}

impl Storage {
    /// Reserves the one buffer a hand-off needs. A refusal of the kernel or of
    /// the allocator maps nothing and is reported as one memory refusal.
    fn reserve() -> Result<Self, Fault> {
        let mut heap = Heap::reserve(1, true).map_err(|_| Fault::Memory)?;
        let block = heap.alloc(MAX_BYTES, 1).map_err(|_| Fault::Memory)?;
        Ok(Self { heap, block })
    }

    /// The bytes of the block, borrowed for as long as a collection may use it.
    fn bytes(&mut self) -> Result<&mut [u8], Fault> {
        self.heap.bytes_mut(&self.block).map_err(|_| Fault::Memory)
    }

    /// Releases the block and unmaps the pages it lived in.
    fn release(mut self) {
        // The run is unmapped by the heap's own `Drop` right after this;
        // releasing the block first keeps the allocator exact while it lasts.
        let _ = self.heap.free(self.block);
    }
}

impl Session<'_> {
    /// Waits for the next step of the owner.
    ///
    /// The owner is the only accepted sender, and a malformed or undeliverable
    /// exchange ends the process instead of being answered as if it were a step.
    fn receive(&self) -> Result<Request, u64> {
        if self.control.wait().is_err() {
            return Err(1);
        }
        let Ok(message) = self.control.receive() else {
            return Err(2);
        };
        if message.sender() != self.owner {
            return Err(3);
        }
        let Ok(words) = k::decode(message.payload()) else {
            return Err(4);
        };
        Ok(Request {
            correlation: message.correlation(),
            words,
        })
    }

    fn answer(&self, request: &Request, reply: [u64; 8]) -> Result<(), u64> {
        self.control
            .send(&Message::new(request.correlation, &k::encode(reply)).unwrap())
            .map_err(|_| 5)
    }
}

/// Serves the tasks-owner protocol until the owner's channel ends.
///
/// The return values are those of the other persistent loops. This loop holds no
/// candidate storage: it answers every step that needs none, and hands the two
/// steps that announce a plan to [`collect`], which owns the pages for exactly
/// as long as the plan does.
pub fn run(files: &mut Files, control: &Endpoint, owner: u64, scope: u32, journal: u32) -> u64 {
    let mut session = Session {
        client: Client::new(Record::object(journal)),
        files,
        control,
        owner,
        scope,
    };
    let mut retained = Retained::default();
    loop {
        let request = match session.receive() {
            Ok(request) => request,
            Err(code) => return code,
        };
        // A begin with no stored edit is refused as out of phase; it may not map a
        // page first, so the refusal reads exactly as it did with fixed storage.
        let needs_storage = request.words[0] == a::TASKS_EDIT
            || (request.words[0] == a::TASKS_BEGIN && retained.has_edit());
        if !needs_storage {
            let mut state = Collection::detached(retained);
            let reply = step(&mut session, &mut state, request.words);
            retained = state.retained();
            if let Err(code) = session.answer(&request, reply) {
                return code;
            }
            continue;
        }
        match Storage::reserve() {
            Ok(mut storage) => {
                let outcome = collect(&mut session, &mut storage, retained, request);
                // The pages go back before the next step is even read, so a
                // query of this process sees no heap between two hand-offs.
                storage.release();
                match outcome {
                    Ok(next) => retained = next,
                    Err(code) => return code,
                }
            }
            // Nothing was mapped and nothing was retained: the step is refused
            // and the client keeps answering with exactly what it already held.
            Err(fault) => {
                let state = Collection::detached(retained);
                let reply = report::step(&state, Some(fault));
                if let Err(code) = session.answer(&request, reply) {
                    return code;
                }
            }
        }
    }
}

/// Serves steps while the client is collecting or holding one plan.
///
/// The plan lives in the storage, so the collection borrows it for as long as it
/// may still need those bytes. Returning is what releases them: it happens as
/// soon as a step leaves the client idle or concludes its apply, and the edit
/// and the concluded apply travel back to the caller, which holds no bytes.
fn collect(
    session: &mut Session<'_>,
    storage: &mut Storage,
    retained: Retained,
    first: Request,
) -> Result<Retained, u64> {
    let bytes = match storage.bytes() {
        Ok(bytes) => bytes,
        Err(fault) => {
            let state = Collection::detached(retained);
            session.answer(&first, report::step(&state, Some(fault)))?;
            return Ok(retained);
        }
    };
    let mut state = Collection::resume(bytes, retained);
    let mut request = first;
    loop {
        let reply = step(session, &mut state, request.words);
        session.answer(&request, reply)?;
        if !state.holds_storage() {
            return Ok(state.retained());
        }
        request = session.receive()?;
    }
}

fn step(session: &mut Session<'_>, state: &mut Collection<'_>, w: [u64; 8]) -> [u64; 8] {
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
        a::TASKS_APPLY => apply(session, state, w[1]),
        a::TASKS_STATUS => {
            let pending = session
                .client
                .pending(&mut Lent {
                    files: &mut *session.files,
                })
                .map_err(Fault::Client);
            report::status(state, pending)
        }
        a::TASKS_RECOVER => {
            let mut trace = Trace::default();
            let result = session
                .client
                .recover(
                    &mut Lent {
                        files: &mut *session.files,
                    },
                    &mut trace,
                )
                .map_err(Fault::Client);
            report::recovery(result, &trace)
        }
        a::TASKS_FORGET => {
            let result = session
                .client
                .forget(
                    &mut Lent {
                        files: &mut *session.files,
                    },
                    w[1],
                )
                .map_err(Fault::Client);
            report::step(state, result.err())
        }
        // The budget walk owns everything it maps and never reads or writes the
        // collection, so every phase admits it and none of them changes.
        a::TASKS_HEAP_STRESS => super::stress::run(),
        _ => report::step(state, Some(Fault::Sequence)),
    }
}

fn apply(session: &mut Session<'_>, state: &mut Collection<'_>, selector: u64) -> [u64; 8] {
    let mut trace = Trace::default();
    // The cut is decided before anything is read, so a selector this build does
    // not implement refuses the step without touching a file.
    let Ok(cut) = cut(selector) else {
        return report::applied(Err(Fault::Sequence), &trace);
    };
    let Some(candidate) = state.candidate() else {
        return report::applied(Err(Fault::Sequence), &trace);
    };
    let result = session
        .client
        .apply_candidate_cut(
            &mut Lent {
                files: &mut *session.files,
            },
            &mut trace,
            session.scope,
            candidate,
            cut,
        )
        .map_err(Fault::Client);
    // A refusal that proves this submission did not publish concludes the
    // attempt; anything else leaves the plan available to the recovery that
    // must resolve it, so the candidate is kept.
    let concluded = match &result {
        Ok(_) => true,
        Err(Fault::Client(error)) => error.conclusive(),
        Err(Fault::Sequence | Fault::Memory) => false,
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
