// SPDX-License-Identifier: Apache-2.0
use super::{content, control, open, record, request};
use crate::volume_disk::Disk;
use core::cell::Cell;
use rustic_file_service::Server7;
use rustic_fs::format7::RecordState;
use rustic_sdk::abi::files::{Error, admission::State, operation};

#[inline(never)]
pub(super) fn verify(disk: &mut Disk<'_>, phase: u64) {
    let volume = open(disk, phase);
    let request = request(volume, 53);
    let mut server = Server7::new(volume);
    if phase == 1 {
        control::admit(&mut server, disk, request, b"unexecuted");
    }
    check(&mut server, disk, request);
}

#[inline(never)]
fn check(server: &mut Server7<'_>, disk: &mut Disk<'_>, request: operation::Replacement) {
    let start = disk.writes;
    let retained = record(server.volume(), request);
    assert_eq!(retained.state, RecordState::Admitted);
    assert_eq!((retained.committed, retained.terminal), (0, 0));
    let mut snapshot = [0; 10];
    assert_eq!(
        server
            .volume()
            .read_retained_range(disk, &retained, 0, &mut snapshot)
            .unwrap(),
        snapshot.len()
    );
    assert_eq!(&snapshot, b"unexecuted");
    let id = rustic_sdk::abi::files::admission::AdmissionId::new(
        server.volume().header().unwrap().lineage,
        retained.admission_number,
    )
    .unwrap();
    let reader = control::grant(server, false);
    let expected = control::status(server, disk, reader, id);
    assert_eq!(expected.state, State::Admitted);

    let caller = control::grant(server, true);
    let count = Cell::new(0);
    assert_eq!(
        control::admit_with_control(server, disk, caller, request, b"unexecuted", &count, |_| 1,),
        Ok(expected)
    );
    let reader = control::grant(server, false);
    assert_eq!(
        control::execute_with_control(server, disk, reader, id, &count, |_| 1),
        Err(rustic_sdk::abi::files::Error::Denied)
    );
    assert_eq!(count.get(), 0);
    assert_eq!(record(server.volume(), request), retained);
    assert_eq!(server.maintain_retention(disk), Err(Error::Busy));
    assert_eq!(
        server
            .volume()
            .stat(request.resource.object())
            .unwrap()
            .version,
        request.expected_version.value()
    );
    content(server.volume(), disk, b"before");
    assert_eq!(disk.writes, start);
}
