// SPDX-License-Identifier: Apache-2.0
use super::super::application::{self, Error};
use super::{Exit, Manager, Memory, State};
use rustic_abi::application as abi;
static ELF: &[u8] = include_bytes!(concat!(
    env!("RUSTIC_APPLICATION_DIRECTORY"),
    "/sdk-probe.elf"
));
static MANIFEST: &[u8] = include_bytes!(concat!(
    env!("RUSTIC_APPLICATION_DIRECTORY"),
    "/app.manifest"
));
pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) {
    let before = memory.free_frames();
    for (offset, expected) in [
        (0, abi::Error::Magic),
        (8, abi::Error::Version),
        (12, abi::Error::Abi),
        (16, abi::Error::Ipc),
        (32, abi::Error::Identity),
        (64, abi::Error::Executable),
        (96, abi::Error::Reserved),
        (31, abi::Error::Capabilities),
    ] {
        let mut bad = [0; abi::SIZE];
        bad.copy_from_slice(MANIFEST);
        bad[offset] = 255;
        assert!(
            matches!(application::launch(manager, memory, &bad, "sdk-probe.elf", ELF, abi::KNOWN), Err(Error::Manifest(e)) if e == expected)
        );
    }
    assert!(matches!(
        application::launch(
            manager,
            memory,
            &MANIFEST[..127],
            "sdk-probe.elf",
            ELF,
            abi::KNOWN
        ),
        Err(Error::Manifest(abi::Error::Length))
    ));
    assert!(matches!(
        application::launch(manager, memory, MANIFEST, "other.elf", ELF, abi::KNOWN),
        Err(Error::Executable)
    ));
    assert!(matches!(
        application::launch(manager, memory, MANIFEST, "sdk-probe.elf", ELF, 0),
        Err(Error::Denied)
    ));
    assert!(matches!(
        application::launch(manager, memory, MANIFEST, "sdk-probe.elf", &[], abi::KNOWN),
        Err(Error::Process(super::super::Error::Elf(_)))
    ));
    assert_eq!(memory.free_frames(), before);
    let a =
        application::launch(manager, memory, MANIFEST, "sdk-probe.elf", ELF, abi::KNOWN).unwrap();
    let b =
        application::launch(manager, memory, MANIFEST, "sdk-probe.elf", ELF, abi::KNOWN).unwrap();
    let peak_frames = before - memory.free_frames();
    // Actual authority comes from these trusted launcher operations, never the manifest.
    let (ha, hb) = manager.connect(a, b).unwrap();
    manager.bootstrap(a, [ha, 0, b.0]);
    manager.bootstrap(b, [hb, 1, a.0]);
    for _ in 0..256 {
        if [a, b]
            .iter()
            .all(|pid| matches!(manager.state(*pid).unwrap(), State::Exited(_)))
        {
            break;
        }
        assert!(manager.step(memory).unwrap().is_some());
    }
    for pid in [a, b] {
        assert_eq!(manager.state(pid).unwrap(), State::Exited(Exit::Code(0)));
        let process = manager.process(pid).unwrap();
        assert_eq!(process.reports, 1);
        assert_eq!(process.last_report, 0x53444b);
        assert_eq!(manager.wait(memory, pid).unwrap(), Some(Exit::Code(0)));
    }
    assert_eq!(manager.broker.counts(), (0, 0));
    assert_eq!(memory.free_frames(), before);
    let mut serial = crate::arch::Serial::take().unwrap();
    use core::fmt::Write;
    writeln!(serial, "RUSTIC SDK verified=1 ring=3 applications=2 exchanges=4 admission_rejected=12 parameters_rejected=4 reports=2 reclaimed=1 elf_bytes={} peak_frames={peak_frames} free_before={before} free_after={}", ELF.len(), memory.free_frames()).unwrap();
    serial.flush();
}
