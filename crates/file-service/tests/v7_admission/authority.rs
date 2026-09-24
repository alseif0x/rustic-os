// SPDX-License-Identifier: Apache-2.0
//! Who may reach a V7 admission and with which request shapes: subjects,
//! scopes, rights profiles and lineage; profile markers; stage kinds that do
//! not cross; retry identities of direct writes; and authority loss during a
//! staged admission.
use super::*;
use rustic_abi::files::REPLACE_ABORT;
use rustic_file_service::READ_ONLY7;

/// One admitted record in the fixture's file, by a subject-2 admission client
/// in slot 0.
fn admitted(
    server: &mut Server7<'_>,
    disk: &mut Sparse,
    (workspace, file): (u32, u32),
) -> (Client, Status) {
    let admission = request(server.volume(), workspace, file, 0x61);
    let client = Client::grant(server, 0, workspace, ADMISSION7, SUBJECT);
    let status = client
        .admit(server, disk, admission, &pattern(1, 3000))
        .unwrap();
    (client, status)
}

#[test]
fn other_subjects_scopes_and_lineages_cannot_see_or_act_on_an_admission() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let (_, accepted) = admitted(&mut server, &mut f.disk, (f.workspace, f.file));
    let quiet = f.disk.mutations();

    let stranger = Client::grant(&mut server, 1, f.workspace, ADMISSION7, 3);
    let outside = Client::grant(&mut server, 2, f.sibling, ADMISSION7, SUBJECT);
    for client in [&stranger, &outside] {
        for op in [a::GET, a::EXECUTE, a::CANCEL] {
            assert_eq!(
                client.action(&mut server, &mut f.disk, op, accepted.id),
                Err(Error::OutcomeUnknown),
                "op {op}"
            );
        }
        assert_eq!(
            client.observe(&mut server, &mut f.disk, accepted.id),
            Err(Error::OutcomeUnknown)
        );
    }
    let reader = Client::grant(&mut server, 3, f.workspace, READ_ONLY7, 0);
    for op in [a::GET, a::EXECUTE, a::CANCEL] {
        assert_eq!(
            reader.action(&mut server, &mut f.disk, op, accepted.id),
            Err(Error::Denied),
            "op {op}"
        );
    }
    let foreign = AdmissionId::new([0x11; 16], accepted.id.number()).unwrap();
    let owner = Client::grant(&mut server, 1, f.workspace, ADMISSION7, SUBJECT);
    assert_eq!(
        owner.action(&mut server, &mut f.disk, a::GET, foreign),
        Err(Error::Lineage)
    );
    let missing = AdmissionId::new(LINEAGE, accepted.id.number() + 50).unwrap();
    assert_eq!(
        owner.action(&mut server, &mut f.disk, a::EXECUTE, missing),
        Err(Error::OutcomeUnknown)
    );
    assert_eq!(f.disk.mutations(), quiet);
    drop(server);
    assert_eq!(retained_state(&f.volume, 0x61), Some(RecordState::Admitted));
}

#[test]
fn read_only_and_out_of_scope_grants_cannot_stage_an_admission() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x62);
    let mut server = Server7::new(&mut f.volume);
    let reader = Client::grant(&mut server, 0, f.workspace, READ_ONLY7, 0);
    let outside = Client::grant(&mut server, 1, f.sibling, ADMISSION7, SUBJECT);
    let quiet = f.disk.mutations();
    for client in [&reader, &outside] {
        assert_eq!(
            client.stage(&mut server, &mut f.disk, admission, b"bytes"),
            Err(Error::Denied)
        );
    }
    assert_eq!(server.volume().open_stages(), 0);
    assert_eq!(f.disk.mutations(), quiet);
}

#[test]
fn unmarked_profile_one_and_live_scheduling_requests_are_unsupported() {
    let mut f = fixture();
    let mut server = Server7::new(&mut f.volume);
    let (client, accepted) = admitted(&mut server, &mut f.disk, (f.workspace, f.file));
    let admission = request(server.volume(), f.workspace, f.file, 0x63);
    let quiet = f.disk.mutations();

    let mut open = admission.packet(10, client.context).unwrap();
    open.op = a::OPEN;
    let mut retry = operation::Lookup::Retry {
        workspace: admission.workspace,
        retry: admission.retry,
    }
    .packet(client.context);
    retry.op = a::RETRY;
    assert_eq!(retry.count, 24);
    let mut requests = vec![open, retry];
    for op in [a::SCHEDULE, a::ACTIVITY, a::REQUEST_CANCEL] {
        requests.push(accepted.id.packet(op, client.context).unwrap());
    }
    for p in requests {
        assert_eq!(
            status(client.send(&mut server, &mut f.disk, p)),
            Err(Error::Unsupported),
            "op {}",
            p.op
        );
    }
    // An unknown observation profile is refused as such.
    let mut observe = accepted.id.packet(a::OBSERVE, client.context).unwrap();
    observe.arg = 3;
    assert_eq!(
        status(client.send(&mut server, &mut f.disk, observe)),
        Err(Error::UnsupportedVersion)
    );
    // A malformed admission ID is a protocol error, not a lookup.
    let mut bad = accepted.id.packet(a::GET, client.context).unwrap();
    bad.arg = 1;
    assert_eq!(
        status(client.send(&mut server, &mut f.disk, bad)),
        Err(Error::Protocol)
    );
    assert_eq!(f.disk.mutations(), quiet);
}

#[test]
fn a_transfer_only_continues_and_finishes_as_the_kind_it_was_opened() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x64);
    let tracked = request(&f.volume, f.workspace, f.file, 0x65);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);

    client
        .stage(&mut server, &mut f.disk, admission, b"admitted")
        .unwrap();
    assert_eq!(
        client.chunks(&mut server, &mut f.disk, REPLACE_CHUNK, f.file, b"x"),
        Err(Error::NoTransfer)
    );
    for op in [REPLACE_COMMIT, REPLACE_ABORT] {
        assert_eq!(
            client.bare(&mut server, &mut f.disk, op, f.file),
            Err(Error::NoTransfer)
        );
    }
    // One transfer per slot, whatever its kind.
    assert_eq!(
        client.stage_tracked(&mut server, &mut f.disk, tracked, b"tracked"),
        Err(Error::Busy)
    );
    client
        .bare(&mut server, &mut f.disk, a::ABORT, f.file)
        .unwrap();
    assert_eq!(server.volume().open_stages(), 0);

    client
        .stage_tracked(&mut server, &mut f.disk, tracked, b"tracked")
        .unwrap();
    assert_eq!(
        client.chunks(&mut server, &mut f.disk, a::CHUNK, f.file, b"x"),
        Err(Error::NoTransfer)
    );
    for op in [a::ACCEPT, a::ABORT] {
        assert_eq!(
            client.bare(&mut server, &mut f.disk, op, f.file),
            Err(Error::NoTransfer)
        );
    }
    client
        .bare(&mut server, &mut f.disk, REPLACE_COMMIT, f.file)
        .unwrap();

    // The committed direct write's retry identity cannot become an admission.
    let reused = operation::Replacement {
        expected_version: tracked.expected_version,
        ..tracked
    };
    assert_eq!(
        client.stage(&mut server, &mut f.disk, reused, b"tracked"),
        Err(Error::IdempotencyConflict)
    );
    // An incomplete admission is refused with `Offset` and stays open.
    let partial = request(server.volume(), f.workspace, f.file, 0x66);
    let mut open = Replacement { request: partial }
        .packet(100, client.context)
        .unwrap();
    open.op = a::OPEN;
    status(client.send(&mut server, &mut f.disk, open)).unwrap();
    client
        .chunks(&mut server, &mut f.disk, a::CHUNK, f.file, &[1; 40])
        .unwrap();
    assert_eq!(
        client.bare(&mut server, &mut f.disk, a::ACCEPT, f.file),
        Err(Error::Offset)
    );
    assert_eq!(server.volume().open_stages(), 1);
}

#[test]
fn revoking_a_staged_admission_releases_its_stage_and_admits_nothing() {
    let mut f = fixture();
    let admission = request(&f.volume, f.workspace, f.file, 0x67);
    let bytes = pattern(2, 5000);
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    client
        .stage(&mut server, &mut f.disk, admission, &bytes[..4000])
        .unwrap();
    server.revoke(0).unwrap();
    assert_eq!(server.volume().open_stages(), 0);
    assert_eq!(
        client.bare(&mut server, &mut f.disk, a::ACCEPT, f.file),
        Err(Error::Revoked)
    );
    // A fresh binding finds no transfer and can admit the same key afresh.
    let fresh = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    assert_eq!(
        fresh.bare(&mut server, &mut f.disk, a::ACCEPT, f.file),
        Err(Error::NoTransfer)
    );
    let accepted = fresh
        .admit(&mut server, &mut f.disk, admission, &bytes)
        .unwrap();
    assert_eq!(accepted.state, State::Admitted);
}

#[test]
fn a_removed_target_denies_execution_but_cancel_still_resolves_the_admission() {
    let mut f = fixture();
    let accepted = {
        let mut server = Server7::new(&mut f.volume);
        admitted(&mut server, &mut f.disk, (f.workspace, f.file)).1
    };
    f.volume.remove(&mut f.disk, f.file).unwrap();
    let mut server = Server7::new(&mut f.volume);
    let client = Client::grant(&mut server, 0, f.workspace, ADMISSION7, SUBJECT);
    let quiet = f.disk.mutations();
    assert_eq!(
        client.action(&mut server, &mut f.disk, a::EXECUTE, accepted.id),
        Err(Error::Denied)
    );
    assert_eq!(f.disk.mutations(), quiet);
    // The record stays visible through its live workspace and can be cancelled.
    assert_eq!(
        client
            .action(&mut server, &mut f.disk, a::GET, accepted.id)
            .unwrap(),
        accepted
    );
    let cancelled = client
        .action(&mut server, &mut f.disk, a::CANCEL, accepted.id)
        .unwrap();
    assert_eq!(cancelled.state, State::Cancelled);
}
