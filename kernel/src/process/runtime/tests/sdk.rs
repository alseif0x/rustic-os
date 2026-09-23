// SPDX-License-Identifier: Apache-2.0
use super::super::application::{self, Error};
use super::{Exit, Manager, Memory, State};
use rustic_abi::application as abi;
use sha2::{Digest, Sha256};
pub(super) static ELF: &[u8] = include_bytes!(concat!(
    env!("RUSTIC_APPLICATION_DIRECTORY"),
    "/sdk-probe.elf"
));
pub(super) static MANIFEST: &[u8] = include_bytes!(concat!(
    env!("RUSTIC_APPLICATION_DIRECTORY"),
    "/app.manifest"
));

pub(super) fn launch_fixture(
    manager: &mut Manager,
    memory: &mut Memory,
) -> rustic_kernel::process::lifecycle::Pid {
    application::launch(manager, memory, MANIFEST, "sdk-probe.elf", ELF, abi::KNOWN).unwrap()
}

pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) {
    let before = memory.free_frames();
    for (offset, expected) in [
        (0, abi::Error::Magic),
        (8, abi::Error::Version),
        (12, abi::Error::Abi),
        (16, abi::Error::Ipc),
        (32, abi::Error::Identity),
        (64, abi::Error::Executable),
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
    let pids_before: [Option<rustic_kernel::process::lifecycle::Pid>;
        rustic_kernel::process::lifecycle::CAPACITY] =
        core::array::from_fn(|slot| manager.table.pid_at(slot));
    let mut mismatched = [0; abi::SIZE];
    mismatched.copy_from_slice(MANIFEST);
    mismatched[96] ^= 1;
    assert!(matches!(
        application::launch(
            manager,
            memory,
            &mismatched,
            "sdk-probe.elf",
            ELF,
            abi::KNOWN
        ),
        Err(Error::ArtifactDigest)
    ));
    assert_eq!(memory.free_frames(), before);
    let pids_after: [Option<rustic_kernel::process::lifecycle::Pid>;
        rustic_kernel::process::lifecycle::CAPACITY] =
        core::array::from_fn(|slot| manager.table.pid_at(slot));
    assert_eq!(pids_after, pids_before);
    let mut empty_elf_manifest = [0; abi::SIZE];
    empty_elf_manifest.copy_from_slice(MANIFEST);
    empty_elf_manifest[96..128].copy_from_slice(&Sha256::digest([]));
    assert!(matches!(
        application::launch(
            manager,
            memory,
            &empty_elf_manifest,
            "sdk-probe.elf",
            &[],
            abi::KNOWN
        ),
        Err(Error::Process(super::super::Error::Elf(_)))
    ));
    assert_eq!(memory.free_frames(), before);
    super::staged::verify(manager, memory);
    let a =
        application::launch(manager, memory, MANIFEST, "sdk-probe.elf", ELF, abi::KNOWN).unwrap();
    let b =
        application::launch(manager, memory, MANIFEST, "sdk-probe.elf", ELF, abi::KNOWN).unwrap();
    let peak_frames = before - memory.free_frames();
    // Actual authority comes from these trusted launcher operations, never the manifest.
    let (ha, hb) = manager.connect(a, b).unwrap();
    manager.bootstrap(a, [ha, 0, b.0]);
    manager.bootstrap(b, [hb, 1, a.0]);
    // The exchange is followed by an independent bounded-memory phase in each
    // application, so the event budget covers more than the four exchanges.
    for _ in 0..1024 {
        if [a, b]
            .iter()
            .all(|pid| matches!(manager.state(*pid).unwrap(), State::Exited(_)))
        {
            break;
        }
        assert!(manager.step(memory).unwrap().is_some());
    }
    let mut summary = None;
    for pid in [a, b] {
        assert_eq!(manager.state(pid).unwrap(), State::Exited(Exit::Code(0)));
        let process = manager.process(pid).unwrap();
        // The exchange marker first, the bounded memory summary last.
        assert_eq!(process.reports, 2);
        let heap = memory_summary(process.last_report);
        assert_eq!(*summary.get_or_insert(heap), heap);
        assert_eq!(manager.wait(memory, pid).unwrap(), Some(Exit::Code(0)));
    }
    let heap = summary.expect("memory summary from both applications");
    assert_eq!(manager.broker.counts(), (0, 0));
    // Both applications mapped and released user pages; nothing may remain.
    assert_eq!(memory.free_frames(), before);
    let mut serial = crate::arch::Serial::take().unwrap();
    use core::fmt::Write;
    writeln!(serial, "RUSTIC SDK verified=1 ring=3 applications=2 exchanges=4 admission_rejected=12 parameters_rejected=4 reports=4 reclaimed=1 elf_bytes={} peak_frames={peak_frames} heap_limit={} heap_peak_pages={} heap_peak_bytes={} heap_full=1 heap_reuse=1 heap_zeroed=1 heap_guarded=1 heap_final_pages=0 staged_images=1 stage_generation_bound=1 stage_duplicate_preserved=1 stage_stale_preserved=1 stage_unauthorized=1 stage_refusal_cleanup=1 stage_abort=1 stage_full=1 stage_digest=1 stage_loader=1 dormant=1 stage_limit={} free_before={before} free_after={}", ELF.len(), heap.limit, heap.peak_pages, heap.peak_bytes, rustic_abi::runtime::MAX_STAGED_IMAGE_BYTES, memory.free_frames()).unwrap();
    serial.flush();
}

/// Bounded dynamic-memory facts the guest observed for itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Summary {
    limit: u64,
    peak_pages: u64,
    peak_bytes: u64,
}

/// Decodes one packed report word; the layout is documented in
/// `apps/sdk-probe/src/heap.rs` and is not an ABI.
fn memory_summary(report: u64) -> Summary {
    assert_eq!(report >> 56, 0x48, "memory summary tag");
    assert_eq!(report >> 32 & 0xff, 0, "user pages still mapped at exit");
    assert_eq!(report & 0xff0, 0, "reserved summary bits");
    assert_eq!(
        report & 0xf,
        0xf,
        "over-limit growth refused, block reused, fresh pages zeroed, foreign unmap refused"
    );
    let summary = Summary {
        limit: report >> 48 & 0xff,
        peak_pages: report >> 40 & 0xff,
        peak_bytes: report >> 12 & 0xf_ffff,
    };
    assert!(summary.peak_pages > 1, "the heap never grew");
    assert!(summary.limit >= summary.peak_pages, "peak above the limit");
    assert!(summary.peak_bytes > 0, "no bytes were allocated");
    summary
}
