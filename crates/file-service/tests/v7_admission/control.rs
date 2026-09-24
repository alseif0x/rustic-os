// SPDX-License-Identifier: Apache-2.0
//! Owner control during an in-flight admission publication: the service
//! drives it over a pollable disk whose every command stays pending for one
//! poll, and the owner revokes, detaches or lets a grant expire between polls.
//! Before the header the publication stops with nothing durable (an execution
//! is then cancelled with `AuthorityLost`); after it the effect settles and
//! stands.
use super::*;
use core::task::Poll;
use rustic_file_service::Control7;
use rustic_fs::{PollDisk, PollDisk7, Publication7Phase};

/// A pollable view of the sparse disk: every publication command is
/// submitted on its first poll and completes on the next poll of the same
/// command, so the owner gets a control opportunity with it outstanding.
/// Blocking commands (stage writes, reads) pass straight through. The
/// command with index `fail` (counting completed ones) completes with an I/O
/// error instead of reaching the disk.
struct Polled<'a> {
    disk: &'a mut Sparse,
    outstanding: Option<(u8, u64)>,
    commands: usize,
    fail: Option<usize>,
}

impl<'a> Polled<'a> {
    fn new(disk: &'a mut Sparse) -> Self {
        Self {
            disk,
            outstanding: None,
            commands: 0,
            fail: None,
        }
    }

    /// Submit the command on its first poll; perform it on the next.
    fn poll(
        &mut self,
        kind: u8,
        sector: u64,
        perform: impl FnOnce(&mut Sparse) -> Result<(), FsError>,
    ) -> Poll<Result<(), FsError>> {
        match self.outstanding {
            Some(command) if command == (kind, sector) => {
                self.outstanding = None;
                let index = self.commands;
                self.commands += 1;
                if self.fail == Some(index) {
                    return Poll::Ready(Err(FsError::Io));
                }
                Poll::Ready(perform(self.disk))
            }
            Some(_) => panic!("a different command was polled while one was outstanding"),
            None => {
                self.outstanding = Some((kind, sector));
                Poll::Pending
            }
        }
    }
}

impl Disk for Polled<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        assert!(
            self.outstanding.is_none(),
            "blocking read under a pending command"
        );
        self.disk.read(sector, bytes)
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
        assert!(
            self.outstanding.is_none(),
            "blocking write under a pending command"
        );
        self.disk.write(sector, bytes)
    }
    fn flush(&mut self) -> Result<(), FsError> {
        assert!(
            self.outstanding.is_none(),
            "blocking flush under a pending command"
        );
        self.disk.flush()
    }
}

impl PollDisk for Polled<'_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), FsError>> {
        let bytes = *bytes;
        self.poll(1, sector, |disk| disk.write(sector, &bytes))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), FsError>> {
        self.poll(2, 0, Sparse::flush)
    }
}

impl PollDisk7 for Polled<'_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), FsError>> {
        let mut read = [0; 512];
        let result = self.poll(0, sector, |disk| disk.read(sector, &mut read));
        if result.is_ready() {
            *bytes = read;
        }
        result
    }
}

/// When the owner acts during the publication.
#[derive(Clone, Copy, PartialEq)]
enum When {
    /// With the first command outstanding, long before the header.
    BeforeHeader,
    /// Once the header may have been submitted.
    AfterHeader,
}

/// What the owner does at that moment.
#[derive(Clone, Copy)]
enum Act {
    Revoke(usize),
    Detach(usize),
    /// Advance the owner's clock to this time.
    Clock(u64),
}

/// The owner's view of one driven request.
#[derive(Default)]
struct Seen {
    callbacks: usize,
    pending: usize,
    acted: Option<Publication7Phase>,
    after_act: usize,
}

/// Send `p` from `client` through `handle_with` over the polled disk; the
/// owner performs `act` once, `when` it first applies.
fn driven(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    client: &Client,
    p: Packet,
    when: When,
    act: Act,
) -> (Packet, Seen) {
    driven_failing(server, disk, client, p, when, act, None)
}

/// [`driven`] with the polled command of index `fail` completing with an I/O
/// error.
fn driven_failing(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    client: &Client,
    mut p: Packet,
    when: When,
    act: Act,
    fail: Option<usize>,
) -> (Packet, Seen) {
    p.context = client.context;
    let mut polled = Polled::new(disk);
    polled.fail = fail;
    let mut seen = Seen::default();
    let mut clock = 0;
    let reply = server.handle_with(
        &mut polled,
        client.slot,
        PEER,
        p,
        0,
        |control: &mut Control7<'_>| {
            seen.callbacks += 1;
            seen.pending += usize::from(control.pending());
            if seen.acted.is_some() {
                seen.after_act += 1;
                return clock;
            }
            let due = match when {
                When::BeforeHeader => control.pending(),
                When::AfterHeader => control.phase() == Publication7Phase::Settling,
            };
            if due {
                seen.acted = Some(control.phase());
                match act {
                    Act::Revoke(slot) => control.revoke(slot).unwrap(),
                    Act::Detach(slot) => control.detach(slot),
                    Act::Clock(now) => clock = now,
                }
            }
            clock
        },
    );
    assert!(
        fail.is_some() || polled.outstanding.is_none(),
        "a command was left outstanding"
    );
    assert_eq!((reply.op, reply.context), (p.op, p.context));
    (reply, seen)
}

/// One admitted record by the slot-0 client and the file's content before it.
fn admitted(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    (workspace, file): (u32, u32),
    key: u64,
) -> (Client, Status, Vec<u8>) {
    let before = read_all(server.volume(), disk, file);
    let admission = request(server.volume(), workspace, file, key);
    let client = Client::grant(server, 0, workspace, ADMISSION7, SUBJECT);
    let status = client
        .admit(server, disk, admission, &pattern(7, 3000))
        .unwrap();
    assert_eq!(status.state, State::Admitted);
    (client, status, before)
}

fn prevention(server: &mut Server7<'_>, disk: &mut Sparse, id: AdmissionId) -> ObservationV2 {
    let fresh = Client::grant(server, 1, 4, ADMISSION7, SUBJECT);
    fresh.observe(server, disk, id).unwrap()
}

#[test]
fn revocation_before_the_header_stops_execution_and_records_authority_lost() {
    let mut f = fixture();
    let file = f.file;
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, before) =
        admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x71);
    let version_before = version(server.volume(), file);
    let execute = accepted.id.packet(a::EXECUTE, 0).unwrap();
    let (reply, seen) = driven(
        &mut server,
        &mut f.disk,
        &client,
        execute,
        When::BeforeHeader,
        Act::Revoke(0),
    );
    assert_eq!(status(reply), Err(Error::Revoked));
    // Revoked with the first command outstanding, before the header.
    assert_eq!(seen.acted, Some(Publication7Phase::Preparing));
    // The owner kept control through the drain and the prevention.
    assert!(seen.pending >= 2 && seen.after_act > 2);

    // The file is unchanged; the admission is cancelled for lost authority.
    assert_eq!(version(server.volume(), file), version_before);
    assert_eq!(read_all(server.volume(), &mut f.disk, file), before);
    assert_eq!(
        retained_state(server.volume(), 0x71),
        Some(RecordState::Cancelled)
    );
    assert_eq!(server.volume().open_stages(), 0);
    let view = prevention(&mut server, &mut f.disk, accepted.id);
    let ObservationV2::Retained {
        status: cancelled,
        prevention: cause,
    } = view
    else {
        panic!("not retained: {view:?}");
    };
    assert_eq!(
        (cancelled.state, cause),
        (State::Cancelled, Some(PreventionReason::AuthorityLost))
    );
    // A later execution on a fresh binding replays the cancellation.
    let fresh = Client::grant(&mut server, 2, f.workspace, ADMISSION7, SUBJECT);
    let quiet = f.disk.mutations();
    assert_eq!(
        fresh
            .action(&mut server, &mut f.disk, a::EXECUTE, accepted.id)
            .unwrap(),
        cancelled
    );
    assert_eq!(f.disk.mutations(), quiet);

    drop(server);
    let mut volume = remount(&mut f.disk);
    assert_eq!(version(&volume, file), version_before);
    assert_eq!(read_all(&volume, &mut f.disk, file), before);
    let mut server = Server7::new(&mut volume);
    assert_eq!(prevention(&mut server, &mut f.disk, accepted.id), view);
}

#[test]
fn revocation_after_the_header_lets_execution_settle_and_withholds_the_reply() {
    let mut f = fixture();
    let file = f.file;
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, _) = admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x72);
    let execute = accepted.id.packet(a::EXECUTE, 0).unwrap();
    let (reply, seen) = driven(
        &mut server,
        &mut f.disk,
        &client,
        execute,
        When::AfterHeader,
        Act::Revoke(0),
    );
    assert_eq!(seen.acted, Some(Publication7Phase::Settling));
    // The effect stands but is not reported under dead authority.
    assert_eq!(status(reply), Err(Error::Uncertain));
    assert_eq!(
        retained_state(server.volume(), 0x72),
        Some(RecordState::AdmittedCommitted)
    );
    assert_eq!(
        read_all(server.volume(), &mut f.disk, file),
        pattern(7, 3000)
    );
    let fresh = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    let committed = fresh
        .action(&mut server, &mut f.disk, a::GET, accepted.id)
        .unwrap();
    assert_eq!(committed.state, State::Committed);
    assert_eq!(version(server.volume(), file), committed.terminal);

    drop(server);
    let mut volume = remount(&mut f.disk);
    assert_eq!(read_all(&volume, &mut f.disk, file), pattern(7, 3000));
    let mut server = Server7::new(&mut volume);
    let fresh = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    assert_eq!(
        fresh
            .action(&mut server, &mut f.disk, a::GET, accepted.id)
            .unwrap(),
        committed
    );
}

#[test]
fn expiry_or_detach_before_the_header_is_lost_authority_too() {
    for (act, error) in [
        (Act::Clock(500), Error::Expired),
        (Act::Detach(0), Error::Denied),
    ] {
        let mut f = fixture();
        let mut server = Server7::new(&mut f.volume);
        let before = read_all(server.volume(), &mut f.disk, f.file);
        let admission = request(server.volume(), f.workspace, f.file, 0x73);
        let grant = server
            .grant(
                0,
                GrantRequest7 {
                    peer: PEER,
                    endpoint: 90,
                    scope: f.workspace,
                    rights: ADMISSION7,
                    subject: SUBJECT,
                    expires: 400,
                },
            )
            .unwrap();
        let client = Client {
            slot: 0,
            context: grant.context,
        };
        let accepted = client
            .admit(&mut server, &mut f.disk, admission, &pattern(8, 900))
            .unwrap();
        let execute = accepted.id.packet(a::EXECUTE, 0).unwrap();
        let (reply, _) = driven(
            &mut server,
            &mut f.disk,
            &client,
            execute,
            When::BeforeHeader,
            act,
        );
        assert_eq!(status(reply), Err(error));
        assert_eq!(read_all(server.volume(), &mut f.disk, f.file), before);
        let ObservationV2::Retained {
            prevention: cause, ..
        } = prevention(&mut server, &mut f.disk, accepted.id)
        else {
            panic!("not retained");
        };
        assert_eq!(cause, Some(PreventionReason::AuthorityLost));
    }
}

#[test]
fn revocation_before_the_header_of_an_acceptance_leaves_no_admission() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let before = read_all(server.volume(), &mut f.disk, f.file);
    let admission = request(server.volume(), f.workspace, f.file, 0x74);
    let bytes = pattern(9, 5000);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    client
        .stage(&mut server, &mut f.disk, admission, &bytes)
        .unwrap();
    let sequence = server.volume().header().unwrap().sequence;
    let mut accept = Packet::new(a::ACCEPT);
    accept.id = f.file;
    let (reply, seen) = driven(
        &mut server,
        &mut f.disk,
        &client,
        accept,
        When::BeforeHeader,
        Act::Revoke(0),
    );
    assert_eq!(status(reply), Err(Error::Revoked));
    assert_eq!(seen.acted, Some(Publication7Phase::Preparing));
    // Nothing was published and the stage is gone.
    assert_eq!(server.volume().header().unwrap().sequence, sequence);
    assert_eq!(retained_state(server.volume(), 0x74), None);
    assert_eq!(server.volume().open_stages(), 0);
    assert_eq!(read_all(server.volume(), &mut f.disk, f.file), before);
    // A fresh binding does not see it and can admit the same key afresh.
    let fresh = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    assert_eq!(
        fresh.retry(
            &mut server,
            &mut f.disk,
            admission.workspace,
            admission.retry
        ),
        Err(Error::OutcomeUnknown)
    );
    let again = fresh
        .admit(&mut server, &mut f.disk, admission, &bytes)
        .unwrap();
    assert_eq!(
        (again.state, again.id.number()),
        (State::Admitted, sequence + 1)
    );
}

#[test]
fn revocation_after_the_header_of_an_acceptance_retires_the_admission() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let before = read_all(server.volume(), &mut f.disk, f.file);
    let admission = request(server.volume(), f.workspace, f.file, 0x75);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    client
        .stage(&mut server, &mut f.disk, admission, &pattern(10, 700))
        .unwrap();
    let mut accept = Packet::new(a::ACCEPT);
    accept.id = f.file;
    let (reply, seen) = driven(
        &mut server,
        &mut f.disk,
        &client,
        accept,
        When::AfterHeader,
        Act::Revoke(0),
    );
    assert_eq!(seen.acted, Some(Publication7Phase::Settling));
    assert_eq!(status(reply), Err(Error::Revoked));
    // Admitted, then retired: it can never execute.
    let fresh = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    let retired = fresh
        .retry(
            &mut server,
            &mut f.disk,
            admission.workspace,
            admission.retry,
        )
        .unwrap();
    assert_eq!(retired.state, State::Cancelled);
    let ObservationV2::Retained {
        prevention: cause, ..
    } = fresh.observe(&mut server, &mut f.disk, retired.id).unwrap()
    else {
        panic!("not retained");
    };
    assert_eq!(cause, Some(PreventionReason::AuthorityLost));
    assert_eq!(read_all(server.volume(), &mut f.disk, f.file), before);
    assert_eq!(server.volume().open_stages(), 0);
}

#[test]
fn revocation_before_the_header_of_a_cancellation_keeps_the_admission() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, _) = admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x76);
    let cancel = accepted.id.packet(a::CANCEL, 0).unwrap();
    let (reply, _) = driven(
        &mut server,
        &mut f.disk,
        &client,
        cancel,
        When::BeforeHeader,
        Act::Revoke(0),
    );
    assert_eq!(status(reply), Err(Error::Revoked));
    assert_eq!(
        retained_state(server.volume(), 0x76),
        Some(RecordState::Admitted)
    );
    let fresh = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    assert_eq!(
        fresh
            .action(&mut server, &mut f.disk, a::GET, accepted.id)
            .unwrap(),
        accepted
    );
}

#[test]
fn another_slot_revoked_mid_publication_loses_its_stage_after_settlement() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, _) = admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x77);
    // Slot 1 holds an open tracked transfer on another file.
    let other = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    let mut open = Replacement {
        request: request(server.volume(), f.workspace, f.sibling, 0x78),
    }
    .packet(2000, other.context)
    .unwrap();
    open.op = rustic_abi::files::REPLACE_OPEN;
    status(other.send(&mut server, &mut f.disk, open)).unwrap();
    assert_eq!(server.volume().open_stages(), 1);

    let execute = accepted.id.packet(a::EXECUTE, 0).unwrap();
    let (reply, seen) = driven(
        &mut server,
        &mut f.disk,
        &client,
        execute,
        When::BeforeHeader,
        Act::Revoke(1),
    );
    assert!(seen.acted.is_some());
    // The caller kept its authority: execution commits and is reported.
    let executed = Status::decode(&status(reply).unwrap()).unwrap();
    assert_eq!(executed.state, State::Committed);
    // The revoked slot's stage was released once the volume was free.
    assert_eq!(server.volume().open_stages(), 0);
    let mut chunk = Packet::new(REPLACE_CHUNK);
    chunk.id = f.sibling;
    chunk.count = 1;
    assert_eq!(
        status(other.send(&mut server, &mut f.disk, chunk)),
        Err(Error::Revoked)
    );
}

#[test]
fn owner_control_runs_only_while_a_publication_is_in_flight() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, _) = admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x79);
    let mut polled = Polled::new(&mut f.disk);
    let mut get = accepted.id.packet(a::GET, 0).unwrap();
    get.context = client.context;
    let reply = server.handle_with(&mut polled, 0, PEER, get, 0, |_: &mut Control7<'_>| {
        panic!("a lookup called owner control")
    });
    assert_eq!(Status::decode(&status(reply).unwrap()).unwrap(), accepted);
    assert_eq!(polled.commands, 0);
}

#[test]
fn a_failed_prevention_after_a_pre_header_revocation_is_uncertain_and_fences() {
    let mut f = fixture();
    let (file, sibling) = (f.file, f.sibling);
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, before) =
        admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x7a);
    // Slot 1 caches the receipt of an executed admission on the sibling.
    let other = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    let sibling_admission = request(server.volume(), f.workspace, sibling, 0x7b);
    let staged = other
        .admit(
            &mut server,
            &mut f.disk,
            sibling_admission,
            &pattern(3, 600),
        )
        .unwrap();
    let executed = other
        .action(&mut server, &mut f.disk, a::EXECUTE, staged.id)
        .unwrap();
    let completion = executed.completion().unwrap();
    other.receipt(&mut server, &mut f.disk, completion).unwrap();
    let part = || {
        let mut p = Lookup {
            query: operation::Lookup::Id(completion),
        }
        .packet(0);
        p.op = OPERATION_PART;
        p.arg = 40;
        p
    };
    status(other.send(&mut server, &mut f.disk, part())).unwrap();

    // The execution's first command drains (index 0); the prevention's first
    // command (index 1) fails.
    let execute = accepted.id.packet(a::EXECUTE, 0).unwrap();
    let (reply, _) = driven_failing(
        &mut server,
        &mut f.disk,
        &client,
        execute,
        When::BeforeHeader,
        Act::Revoke(0),
        Some(1),
    );
    assert_eq!(status(reply), Err(Error::Uncertain));
    assert!(
        server.volume().header().is_err(),
        "the volume is not fenced"
    );
    // Cached receipts are forgotten and nothing else is served until remount.
    assert_eq!(
        status(other.send(&mut server, &mut f.disk, part())),
        Err(Error::OutcomeUnknown)
    );
    assert!(
        other
            .action(&mut server, &mut f.disk, a::GET, accepted.id)
            .is_err()
    );

    drop(server);
    let volume = remount(&mut f.disk);
    assert_eq!(retained_state(&volume, 0x7a), Some(RecordState::Admitted));
    assert_eq!(read_all(&volume, &mut f.disk, file), before);
}

#[test]
fn a_drained_command_that_fails_during_the_stop_is_uncertain_and_fences() {
    let mut f = fixture();
    let file = f.file;
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, before) =
        admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x7c);
    let execute = accepted.id.packet(a::EXECUTE, 0).unwrap();
    let (reply, seen) = driven_failing(
        &mut server,
        &mut f.disk,
        &client,
        execute,
        When::BeforeHeader,
        Act::Revoke(0),
        Some(0),
    );
    assert_eq!(seen.acted, Some(Publication7Phase::Preparing));
    assert_eq!(status(reply), Err(Error::Uncertain));
    assert!(
        server.volume().header().is_err(),
        "the volume is not fenced"
    );

    drop(server);
    let volume = remount(&mut f.disk);
    assert_eq!(retained_state(&volume, 0x7c), Some(RecordState::Admitted));
    assert_eq!(read_all(&volume, &mut f.disk, file), before);
}

#[test]
fn revocation_after_the_header_of_a_cancellation_lets_it_stand() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, _) = admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x7d);
    let cancel = accepted.id.packet(a::CANCEL, 0).unwrap();
    let (reply, seen) = driven(
        &mut server,
        &mut f.disk,
        &client,
        cancel,
        When::AfterHeader,
        Act::Revoke(0),
    );
    assert_eq!(seen.acted, Some(Publication7Phase::Settling));
    assert_eq!(status(reply), Err(Error::Uncertain));
    assert_eq!(
        retained_state(server.volume(), 0x7d),
        Some(RecordState::Cancelled)
    );
    let ObservationV2::Retained {
        prevention: cause, ..
    } = prevention(&mut server, &mut f.disk, accepted.id)
    else {
        panic!("not retained");
    };
    assert_eq!(cause, Some(PreventionReason::Requested));
}

#[test]
fn a_replayed_acceptance_settles_without_owner_control_or_io() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted, _) = admitted(&mut server, &mut f.disk, (f.workspace, f.file), 0x7e);
    // The exact retry verifies its bytes against the snapshot while staging.
    let admission = request(server.volume(), f.workspace, f.file, 0x7e);
    client
        .stage(&mut server, &mut f.disk, admission, &pattern(7, 3000))
        .unwrap();
    let quiet = f.disk.mutations();
    let mut accept = Packet::new(a::ACCEPT);
    accept.id = f.file;
    accept.context = client.context;
    let mut polled = Polled::new(&mut f.disk);
    let reply = server.handle_with(&mut polled, 0, PEER, accept, 0, |_: &mut Control7<'_>| {
        panic!("a replay called owner control")
    });
    assert_eq!(polled.commands, 0);
    assert_eq!(Status::decode(&status(reply).unwrap()).unwrap(), accepted);
    assert_eq!(f.disk.mutations(), quiet);
}
