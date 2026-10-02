// SPDX-License-Identifier: Apache-2.0
//! V7 lifecycle tickets are volatile, scoped to one service incarnation and
//! advance one durable admission per scheduler turn.
#[path = "v7_scheduling/support.rs"]
mod support;
use support::*;

use rustic_abi::files::{
    Error,
    admission::{self as a, Activity, ActivityPhase, AdmissionId, ObservationV2, State, Status},
    lifecycle::{CancelAck, Disposition},
};
use rustic_file_service::{ADMISSION7, GrantRequest7, Server7};
use rustic_fs::{Kind, PreventionReason, Volume7};

fn new_server(
    workspace: u32,
    volume: &mut Volume7,
    slot: usize,
    rights: u8,
    subject: u64,
    expires: u64,
) -> (Server7<'_>, rustic_file_service::Grant7) {
    let mut server = Server7::new(volume);
    let grant = grant(&mut server, slot, workspace, rights, subject, expires);
    (server, grant)
}

fn file(fixture: &mut Fixture, name: &[u8]) -> u32 {
    fixture
        .volume
        .create(&mut fixture.disk, fixture.workspace, name, Kind::File)
        .unwrap()
        .id
}

fn id(status: Status) -> AdmissionId {
    status.id
}

#[test]
fn eight_record_inventory_and_fifo_queue_process_exactly_one_ticket_per_turn() {
    let mut fixture = fixture();
    let objects: Vec<u32> = (0..8)
        .map(|index| {
            let name = [b'f', b'0' + index];
            file(&mut fixture, &name)
        })
        .collect();
    let (mut server, executor) = new_server(
        fixture.workspace,
        &mut fixture.volume,
        0,
        rights(),
        SUBJECT,
        0,
    );
    let mut admissions = Vec::new();
    for (index, object) in objects.iter().copied().enumerate() {
        admissions.push(admit(
            &mut server,
            &mut fixture.disk,
            executor,
            fixture.workspace,
            object,
            100 + index as u64,
            b"new",
        ));
    }

    // The final retained slots prove the inventory scans all eight V7 records.
    assert_eq!(
        schedule(&mut server, &mut fixture.disk, executor, id(admissions[6])).status,
        0
    );
    assert_eq!(
        schedule(&mut server, &mut fixture.disk, executor, id(admissions[7])).status,
        0
    );
    error(
        schedule(&mut server, &mut fixture.disk, executor, id(admissions[0])),
        Error::Busy,
    );
    assert!(server.has_scheduled());

    assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
    assert_eq!(
        admission_status(&mut server, &mut fixture.disk, executor, id(admissions[6])).state,
        State::Committed
    );
    assert_eq!(
        admission_status(&mut server, &mut fixture.disk, executor, id(admissions[7])).state,
        State::Admitted
    );
    assert!(server.has_scheduled());

    assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
    assert_eq!(
        admission_status(&mut server, &mut fixture.disk, executor, id(admissions[7])).state,
        State::Committed
    );
    assert!(!server.has_scheduled());
    assert!(!server.run_scheduled(&mut fixture.disk, 0, |_| 0));
}

#[test]
fn sync_execute_exposes_live_activity_and_honors_a_stop_before_header() {
    let mut fixture = fixture();
    let object = file(&mut fixture, b"sync");
    let (mut server, executor) = new_server(
        fixture.workspace,
        &mut fixture.volume,
        0,
        rights(),
        SUBJECT,
        0,
    );
    let admitted = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        201,
        b"sync-data",
    );
    let mut disk = Deferred::new(core::mem::take(&mut fixture.disk), 1);
    let signals = disk.signals.clone();
    let mut saw_running = false;
    let mut saw_pending_stop = false;
    let mut saw_prevention = false;
    let execute = id(admitted).packet(a::EXECUTE, executor.context).unwrap();
    let response = server.handle_with(
        &mut disk,
        slot_for(executor),
        executor.peer,
        execute,
        0,
        |control| {
            if !saw_running && !control.pending() {
                let view = ObservationV2::decode(&control.request(
                    slot_for(executor),
                    executor.peer,
                    ObservationV2::request(id(admitted), executor.context).unwrap(),
                    0,
                ))
                .unwrap();
                assert_active(view, ActivityPhase::Running, false, false);
                saw_running = true;
            }
            if control.pending() && !saw_pending_stop {
                assert_eq!(control.phase(), rustic_fs::Publication7Phase::Preparing);
                let activity = Activity::decode(&control.request(
                    slot_for(executor),
                    executor.peer,
                    request_cancel_packet(id(admitted), executor.context),
                    0,
                ))
                .unwrap();
                assert_eq!(activity.phase, ActivityPhase::Stopping);
                assert!(activity.cancel_requested);
                let view = ObservationV2::decode(&control.request(
                    slot_for(executor),
                    executor.peer,
                    ObservationV2::request(id(admitted), executor.context).unwrap(),
                    0,
                ))
                .unwrap();
                assert_active(view, ActivityPhase::Stopping, true, true);
                saw_pending_stop = true;
                signals.release.set(true);
            }
            if saw_pending_stop
                && signals.calls.get() > 1
                && control.phase() == rustic_fs::Publication7Phase::Preparing
            {
                let view = ObservationV2::decode(&control.request(
                    slot_for(executor),
                    executor.peer,
                    ObservationV2::request(id(admitted), executor.context).unwrap(),
                    0,
                ))
                .unwrap();
                assert_active(view, ActivityPhase::Stopping, true, control.pending());
                saw_prevention = true;
            }
            0
        },
    );
    let result = Status::decode(&good(response)).unwrap();
    assert_eq!(result.state, State::Cancelled);
    assert!(saw_running && saw_pending_stop && saw_prevention);
    let record = server
        .volume()
        .retained_records()
        .unwrap()
        .iter()
        .flatten()
        .find(|record| record.admission_number == id(admitted).number())
        .unwrap();
    assert_eq!(record.prevention, Some(PreventionReason::Requested));
    assert_eq!(server.volume().node(object).unwrap().unwrap().length, 0);
}

#[test]
fn scheduled_stop_after_header_settles_the_committed_effect() {
    let mut fixture = fixture();
    let object = file(&mut fixture, b"late-stop");
    let (mut server, executor) = new_server(
        fixture.workspace,
        &mut fixture.volume,
        0,
        rights(),
        SUBJECT,
        0,
    );
    let admitted = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        202,
        b"committed",
    );
    good(schedule(
        &mut server,
        &mut fixture.disk,
        executor,
        id(admitted),
    ));
    let mut requested = false;
    assert!(server.run_scheduled(&mut fixture.disk, 0, |control| {
        if !requested && control.phase() == rustic_fs::Publication7Phase::Settling {
            let ack = CancelAck::decode(&control.request(
                slot_for(executor),
                executor.peer,
                lifecycle_cancel_packet(id(admitted), executor.context),
                0,
            ))
            .unwrap();
            assert_eq!(ack.disposition, Disposition::Requested);
            let view = ObservationV2::decode(&control.request(
                slot_for(executor),
                executor.peer,
                ObservationV2::request(id(admitted), executor.context).unwrap(),
                0,
            ))
            .unwrap();
            assert_active(view, ActivityPhase::Settling, true, control.pending());
            requested = true;
        }
        0
    }));
    assert!(requested);
    assert_eq!(
        admission_status(&mut server, &mut fixture.disk, executor, id(admitted)).state,
        State::Committed
    );
    assert_eq!(server.volume().node(object).unwrap().unwrap().length, 9);
    assert!(!server.has_scheduled());
}

#[test]
fn simultaneous_stop_and_executor_revocation_persist_authority_lost() {
    let mut fixture = fixture();
    let object = file(&mut fixture, b"authority");
    let (mut server, executor) = new_server(
        fixture.workspace,
        &mut fixture.volume,
        0,
        rights(),
        SUBJECT,
        0,
    );
    let observer = grant(&mut server, 1, fixture.workspace, ADMISSION7, SUBJECT, 0);
    let admitted = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        203,
        b"must-not-run",
    );
    good(schedule(
        &mut server,
        &mut fixture.disk,
        executor,
        id(admitted),
    ));

    let mut disk = Deferred::new(core::mem::take(&mut fixture.disk), 1);
    let signals = disk.signals.clone();
    let mut accepted_stop = false;
    assert!(server.run_scheduled(&mut disk, 0, |control| {
        if control.pending() && !accepted_stop {
            let ack = CancelAck::decode(&control.request(
                slot_for(executor),
                executor.peer,
                lifecycle_cancel_packet(id(admitted), executor.context),
                0,
            ))
            .unwrap();
            assert_eq!(ack.disposition, Disposition::Requested);
            control.revoke(slot_for(executor)).unwrap();
            accepted_stop = true;
            signals.release.set(true);
        }
        0
    }));
    assert!(accepted_stop);
    let mut sparse = disk.inner;
    let view = observation(&mut server, &mut sparse, observer, id(admitted), 0);
    assert_state(
        view,
        State::Cancelled,
        Some(a::PreventionReason::AuthorityLost),
    );
    assert_eq!(server.volume().node(object).unwrap().unwrap().length, 0);
}

#[test]
fn queued_version_conflict_is_terminally_prevented_without_executing_twice() {
    let mut fixture = fixture();
    let object = file(&mut fixture, b"version");
    let (mut server, executor) = new_server(
        fixture.workspace,
        &mut fixture.volume,
        0,
        rights(),
        SUBJECT,
        0,
    );
    let first = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        204,
        b"first",
    );
    let second = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        205,
        b"second",
    );
    good(schedule(
        &mut server,
        &mut fixture.disk,
        executor,
        id(first),
    ));
    good(schedule(
        &mut server,
        &mut fixture.disk,
        executor,
        id(second),
    ));
    assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
    assert_eq!(
        admission_status(&mut server, &mut fixture.disk, executor, id(first)).state,
        State::Committed
    );
    assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
    assert_state(
        observation(&mut server, &mut fixture.disk, executor, id(second), 0),
        State::Cancelled,
        Some(a::PreventionReason::VersionConflict),
    );
    assert_eq!(server.volume().node(object).unwrap().unwrap().length, 5);
}

#[test]
fn cancel_only_scope_is_private_and_accepted_cancel_is_service_owned() {
    let mut fixture = fixture();
    let object = file(&mut fixture, b"cancel-only");
    let (mut server, executor) = new_server(
        fixture.workspace,
        &mut fixture.volume,
        0,
        rights(),
        SUBJECT,
        0,
    );
    let admitted = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        206,
        b"unchanged",
    );
    let wrong_subject = grant(
        &mut server,
        2,
        fixture.workspace,
        rustic_abi::files::CANCEL_RIGHT,
        SUBJECT + 1,
        0,
    );
    error(
        {
            let request = CancelAck::request(id(admitted), wrong_subject.context).unwrap();
            server.handle(
                &mut fixture.disk,
                slot_for(wrong_subject),
                wrong_subject.peer,
                request,
                0,
            )
        },
        Error::OutcomeUnknown,
    );
    assert!(!server.has_scheduled());

    let cancel_only = grant(
        &mut server,
        1,
        fixture.workspace,
        rustic_abi::files::CANCEL_RIGHT,
        SUBJECT,
        0,
    );
    let ack = lifecycle_cancel(&mut server, &mut fixture.disk, cancel_only, id(admitted), 0);
    assert_eq!(ack.disposition, Disposition::Requested);
    assert!(server.has_scheduled());
    // Losing the accepting CANCEL binding cannot revoke service-owned
    // prevention, either before it starts or during its publication.
    server.revoke(slot_for(cancel_only)).unwrap();
    let mut checked = false;
    assert!(server.run_scheduled(&mut fixture.disk, 0, |control| {
        if !checked {
            let missing = AdmissionId::new(LINEAGE, 999).unwrap();
            for target in [id(admitted), missing] {
                error(
                    control.request(
                        slot_for(cancel_only),
                        cancel_only.peer,
                        lifecycle_cancel_packet(target, cancel_only.context),
                        0,
                    ),
                    Error::Revoked,
                );
                error(
                    control.request(
                        slot_for(wrong_subject),
                        wrong_subject.peer,
                        ObservationV2::request(target, wrong_subject.context).unwrap(),
                        0,
                    ),
                    Error::Denied,
                );
            }
            control.revoke(slot_for(executor)).unwrap();
            checked = true;
        }
        0
    }));
    assert!(checked);
    let observer = grant(&mut server, 3, fixture.workspace, ADMISSION7, SUBJECT, 0);
    assert_state(
        observation(&mut server, &mut fixture.disk, observer, id(admitted), 0),
        State::Cancelled,
        Some(a::PreventionReason::Requested),
    );
    assert_eq!(server.volume().node(object).unwrap().unwrap().length, 0);
}

#[test]
fn direct_execute_cannot_bypass_queued_prevention_or_a_queued_stop() {
    // Lifecycle CANCEL may create a cancellation-only prevention ticket. A
    // synchronous EXECUTE with a broader grant must not bypass that ticket.
    {
        let mut fixture = fixture();
        let object = file(&mut fixture, b"queued-prevent");
        let (mut server, executor) = new_server(
            fixture.workspace,
            &mut fixture.volume,
            0,
            rights(),
            SUBJECT,
            0,
        );
        let cancel_only = grant(
            &mut server,
            1,
            fixture.workspace,
            rustic_abi::files::CANCEL_RIGHT,
            SUBJECT,
            0,
        );
        let observer = grant(&mut server, 2, fixture.workspace, ADMISSION7, SUBJECT, 0);
        let admitted = admit(
            &mut server,
            &mut fixture.disk,
            executor,
            fixture.workspace,
            object,
            212,
            b"must-not-execute",
        );
        assert_eq!(
            lifecycle_cancel(&mut server, &mut fixture.disk, cancel_only, id(admitted), 0)
                .disposition,
            Disposition::Requested
        );

        error(
            server.handle(
                &mut fixture.disk,
                slot_for(executor),
                executor.peer,
                id(admitted).packet(a::EXECUTE, executor.context).unwrap(),
                0,
            ),
            Error::Busy,
        );
        assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
        assert_state(
            observation(&mut server, &mut fixture.disk, observer, id(admitted), 0),
            State::Cancelled,
            Some(a::PreventionReason::Requested),
        );
        assert_eq!(server.volume().node(object).unwrap().unwrap().length, 0);
    }

    // REQUEST_CANCEL on an execution ticket has the same ordering guarantee.
    {
        let mut fixture = fixture();
        let object = file(&mut fixture, b"queued-stop");
        let (mut server, executor) = new_server(
            fixture.workspace,
            &mut fixture.volume,
            0,
            rights(),
            SUBJECT,
            0,
        );
        let observer = grant(&mut server, 1, fixture.workspace, ADMISSION7, SUBJECT, 0);
        let admitted = admit(
            &mut server,
            &mut fixture.disk,
            executor,
            fixture.workspace,
            object,
            213,
            b"must-not-execute",
        );
        good(schedule(
            &mut server,
            &mut fixture.disk,
            executor,
            id(admitted),
        ));
        good(send(
            &mut server,
            &mut fixture.disk,
            executor,
            a::REQUEST_CANCEL,
            id(admitted),
            0,
        ));
        error(
            server.handle(
                &mut fixture.disk,
                slot_for(executor),
                executor.peer,
                id(admitted).packet(a::EXECUTE, executor.context).unwrap(),
                0,
            ),
            Error::Busy,
        );
        assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
        assert_state(
            observation(&mut server, &mut fixture.disk, observer, id(admitted), 0),
            State::Cancelled,
            Some(a::PreventionReason::Requested),
        );
        assert_eq!(server.volume().node(object).unwrap().unwrap().length, 0);
    }
}

#[test]
fn queued_expiry_regrant_group_revocation_and_second_scope_change_lose_saved_binding() {
    // Expiry before execution is persisted as AuthorityLost.
    {
        let mut fixture = fixture();
        let object = file(&mut fixture, b"expiry");
        let (mut server, executor) = new_server(
            fixture.workspace,
            &mut fixture.volume,
            0,
            rights(),
            SUBJECT,
            4,
        );
        let observer = grant(&mut server, 1, fixture.workspace, ADMISSION7, SUBJECT, 0);
        let admitted = admit(
            &mut server,
            &mut fixture.disk,
            executor,
            fixture.workspace,
            object,
            207,
            b"x",
        );
        good(schedule(
            &mut server,
            &mut fixture.disk,
            executor,
            id(admitted),
        ));
        assert!(server.run_scheduled(&mut fixture.disk, 4, |_| 4));
        assert_state(
            observation(&mut server, &mut fixture.disk, observer, id(admitted), 4),
            State::Cancelled,
            Some(a::PreventionReason::AuthorityLost),
        );
    }

    // Replacing the original slot cannot turn a duplicate schedule into a new executor.
    {
        let mut fixture = fixture();
        let object = file(&mut fixture, b"regrant");
        let (mut server, original) = new_server(
            fixture.workspace,
            &mut fixture.volume,
            0,
            rights(),
            SUBJECT,
            0,
        );
        let duplicate = grant(&mut server, 1, fixture.workspace, rights(), SUBJECT, 0);
        let observer = grant(&mut server, 2, fixture.workspace, ADMISSION7, SUBJECT, 0);
        let admitted = admit(
            &mut server,
            &mut fixture.disk,
            original,
            fixture.workspace,
            object,
            208,
            b"x",
        );
        good(schedule(
            &mut server,
            &mut fixture.disk,
            original,
            id(admitted),
        ));
        good(schedule(
            &mut server,
            &mut fixture.disk,
            duplicate,
            id(admitted),
        ));
        let mut replacement = GrantRequest7 {
            peer: original.peer,
            endpoint: 190,
            scope: fixture.workspace,
            rights: rights(),
            subject: SUBJECT,
            expires: 0,
        };
        replacement.peer = original.peer;
        server.grant(0, replacement).unwrap();
        let execute = id(admitted).packet(a::EXECUTE, duplicate.context).unwrap();
        error(
            server.handle(
                &mut fixture.disk,
                slot_for(duplicate),
                duplicate.peer,
                execute,
                0,
            ),
            Error::Busy,
        );
        assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
        assert_state(
            observation(&mut server, &mut fixture.disk, observer, id(admitted), 0),
            State::Cancelled,
            Some(a::PreventionReason::AuthorityLost),
        );
    }

    // Revoking a root fences the original helper ticket in the same group.
    {
        let mut fixture = fixture();
        let object = file(&mut fixture, b"group");
        let (mut server, root) = new_server(
            fixture.workspace,
            &mut fixture.volume,
            0,
            rights(),
            SUBJECT,
            0,
        );
        let helper = server
            .derive(
                1,
                root.context,
                GrantRequest7 {
                    peer: PEER + 1,
                    endpoint: 91,
                    scope: fixture.workspace,
                    rights: rights(),
                    subject: 0,
                    expires: 0,
                },
                0,
            )
            .unwrap();
        let observer = grant(&mut server, 2, fixture.workspace, ADMISSION7, SUBJECT, 0);
        let admitted = admit(
            &mut server,
            &mut fixture.disk,
            helper,
            fixture.workspace,
            object,
            209,
            b"x",
        );
        good(schedule(
            &mut server,
            &mut fixture.disk,
            helper,
            id(admitted),
        ));
        server.revoke(0).unwrap();
        assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
        assert_state(
            observation(&mut server, &mut fixture.disk, observer, id(admitted), 0),
            State::Cancelled,
            Some(a::PreventionReason::AuthorityLost),
        );
    }

    // A same-context extension changes the saved binding even though the
    // primary scope still reaches the admitted file.
    {
        let mut fixture = fixture();
        let object = file(&mut fixture, b"second-scope");
        let side_file = fixture
            .volume
            .create(&mut fixture.disk, 4, b"side-file", Kind::File)
            .unwrap()
            .id;
        let (mut server, executor) = new_server(
            fixture.workspace,
            &mut fixture.volume,
            0,
            rights(),
            SUBJECT,
            0,
        );
        let observer = grant(&mut server, 1, fixture.workspace, ADMISSION7, SUBJECT, 0);
        let admitted = admit(
            &mut server,
            &mut fixture.disk,
            executor,
            fixture.workspace,
            object,
            210,
            b"x",
        );
        good(schedule(
            &mut server,
            &mut fixture.disk,
            executor,
            id(admitted),
        ));
        server.extend(0, executor.context, side_file).unwrap();
        assert!(server.run_scheduled(&mut fixture.disk, 0, |_| 0));
        assert_state(
            observation(&mut server, &mut fixture.disk, observer, id(admitted), 0),
            State::Cancelled,
            Some(a::PreventionReason::AuthorityLost),
        );
    }
}

#[test]
fn service_restart_drops_schedules_and_execution_io_failure_fences_the_old_instance() {
    // A queue is never reconstructed from an admitted durable record.
    let mut fixture = fixture();
    let object = file(&mut fixture, b"restart");
    let mut server = Server7::new(&mut fixture.volume);
    let executor = grant(&mut server, 0, fixture.workspace, rights(), SUBJECT, 0);
    let admitted = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        211,
        b"x",
    );
    good(schedule(
        &mut server,
        &mut fixture.disk,
        executor,
        id(admitted),
    ));
    drop(server);
    let mut remounted = remount(&mut fixture.disk);
    let mut restarted = Server7::new(&mut remounted);
    let observer = grant(&mut restarted, 0, fixture.workspace, ADMISSION7, SUBJECT, 0);
    assert!(!restarted.has_scheduled());
    assert_eq!(
        admission_status(&mut restarted, &mut fixture.disk, observer, id(admitted)).state,
        State::Admitted
    );
    assert_eq!(restarted.volume().node(object).unwrap().unwrap().length, 0);

    // A failed command during a held execution fences that incarnation and
    // clears every volatile ticket; remount chooses the untouched admitted head.
    drop(restarted);
    let mut remounted = remount(&mut fixture.disk);
    let mut server = Server7::new(&mut remounted);
    let executor = grant(&mut server, 0, fixture.workspace, rights(), SUBJECT, 0);
    good(schedule(
        &mut server,
        &mut fixture.disk,
        executor,
        id(admitted),
    ));
    let mut disk = Deferred::new(core::mem::take(&mut fixture.disk), 1);
    let signals = disk.signals.clone();
    let mut failed = false;
    assert!(server.run_scheduled(&mut disk, 0, |control| {
        if control.pending() && !failed {
            signals.fail.set(true);
            signals.release.set(true);
            failed = true;
        }
        0
    }));
    assert!(failed);
    assert!(!server.has_scheduled());
    assert!(server.volume().header().is_err());
    drop(server);
    let mut disk = disk.inner;
    let mut remounted = remount(&mut disk);
    let mut restarted = Server7::new(&mut remounted);
    let observer = grant(&mut restarted, 0, fixture.workspace, ADMISSION7, SUBJECT, 0);
    assert!(!restarted.has_scheduled());
    assert_eq!(
        admission_status(&mut restarted, &mut disk, observer, id(admitted)).state,
        State::Admitted
    );
    assert_eq!(restarted.volume().node(object).unwrap().unwrap().length, 0);
}

#[test]
fn failed_requested_prevention_discards_both_tickets_and_remounts_admitted() {
    let mut fixture = fixture();
    let object = file(&mut fixture, b"prevention-failure");
    let mut server = Server7::new(&mut fixture.volume);
    let executor = grant(&mut server, 0, fixture.workspace, rights(), SUBJECT, 0);
    let first = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        214,
        b"first",
    );
    let second = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        215,
        b"second",
    );
    for admitted in [first, second] {
        good(schedule(
            &mut server,
            &mut fixture.disk,
            executor,
            id(admitted),
        ));
    }
    let before = server.volume().header().unwrap().sequence;
    // Cancel before the execution's first command. The pending command then
    // belongs to the service-owned Requested prevention, which fails.
    let mut disk = Deferred::new(core::mem::take(&mut fixture.disk), 1);
    let signals = disk.signals.clone();
    let mut stopped = false;
    let mut failed_prevention = false;
    assert!(server.run_scheduled(&mut disk, 0, |control| {
        if !stopped {
            let ack = CancelAck::decode(&control.request(
                slot_for(executor),
                executor.peer,
                lifecycle_cancel_packet(id(first), executor.context),
                0,
            ))
            .unwrap();
            assert_eq!(ack.disposition, Disposition::Requested);
            stopped = true;
        } else if control.pending() && !failed_prevention {
            let view = ObservationV2::decode(&control.request(
                slot_for(executor),
                executor.peer,
                ObservationV2::request(id(first), executor.context).unwrap(),
                0,
            ))
            .unwrap();
            assert_active(view, ActivityPhase::Stopping, true, true);
            signals.fail.set(true);
            signals.release.set(true);
            failed_prevention = true;
        }
        0
    }));
    assert!(stopped && failed_prevention);
    assert!(!server.has_scheduled());
    assert!(server.volume().header().is_err());
    assert!(!server.run_scheduled(&mut disk, 0, |_| panic!("fenced queue ran")));
    drop(server);
    let mut disk = disk.inner;
    let mut volume = remount(&mut disk);
    assert_eq!(volume.header().unwrap().sequence, before);
    let mut server = Server7::new(&mut volume);
    let observer = grant(&mut server, 0, fixture.workspace, ADMISSION7, SUBJECT, 0);
    for admitted in [first, second] {
        assert_state(
            observation(&mut server, &mut disk, observer, id(admitted), 0),
            State::Admitted,
            None,
        );
    }
    assert_eq!(server.volume().node(object).unwrap().unwrap().length, 0);
}

#[test]
fn an_ordinary_namespace_failure_also_discards_the_volatile_queue() {
    use rustic_abi::files::{CREATE, Packet};
    use rustic_fs::{Disk, Error as FsError};
    struct FailedWrite<'a>(&'a mut Sparse);
    impl Disk for FailedWrite<'_> {
        fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
            self.0.read(sector, bytes)
        }
        fn write(&mut self, _: u64, _: &[u8; 512]) -> Result<(), FsError> {
            Err(FsError::Io)
        }
        fn flush(&mut self) -> Result<(), FsError> {
            Err(FsError::Io)
        }
    }
    let mut fixture = fixture();
    let object = file(&mut fixture, b"queued-before-failure");
    let mut server = Server7::new(&mut fixture.volume);
    let executor = grant(&mut server, 0, fixture.workspace, rights(), SUBJECT, 0);
    let admitted = admit(
        &mut server,
        &mut fixture.disk,
        executor,
        fixture.workspace,
        object,
        216,
        b"not-run",
    );
    good(schedule(
        &mut server,
        &mut fixture.disk,
        executor,
        id(admitted),
    ));
    let mut create = Packet::new(CREATE);
    create.context = executor.context;
    create.id = fixture.workspace;
    create.count = 6;
    create.data[..6].copy_from_slice(b"broken");
    error(
        server.handle(
            &mut FailedWrite(&mut fixture.disk),
            slot_for(executor),
            executor.peer,
            create,
            0,
        ),
        Error::Uncertain,
    );
    assert!(server.volume().header().is_err());
    assert!(!server.has_scheduled());
    assert!(!server.run_scheduled(&mut fixture.disk, 0, |_| panic!("fenced queue ran")));
    drop(server);
    let mut volume = remount(&mut fixture.disk);
    let mut server = Server7::new(&mut volume);
    let observer = grant(&mut server, 0, fixture.workspace, rights(), SUBJECT, 0);
    assert_state(
        observation(&mut server, &mut fixture.disk, observer, id(admitted), 0),
        State::Admitted,
        None,
    );
    assert_eq!(server.volume().node(object).unwrap().unwrap().length, 0);
}
