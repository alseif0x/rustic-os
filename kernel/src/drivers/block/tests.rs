// SPDX-License-Identifier: Apache-2.0
use super::Device;
use crate::arch::{Serial, memory::Memory};
use core::fmt::Write;
use rustic_kernel::{
    block::{Error, SECTOR},
    boot::BootMode,
};
const CAPACITY: u64 = 8_388_608;
fn pattern(sector: u64) -> [u8; SECTOR] {
    core::array::from_fn(|i| ((sector.wrapping_mul(17) + i as u64 * 29) % 251 + 1) as u8)
}
pub(crate) fn verify(memory: &mut Memory, mode: BootMode) {
    let before = memory.free_frames();
    if mode == BootMode::BlockMissing {
        assert!(matches!(Device::open(memory), Err(Error::Missing)));
        emit("missing", before, memory.free_frames(), 0);
        return;
    }
    memory.verify_frame_budget(2, |memory| {
        assert!(matches!(Device::open(memory), Err(Error::Memory)));
    });
    let mut device = Device::open(memory).unwrap();
    let dma_frames = before - memory.free_frames();
    assert_eq!(device.geometry().sectors, CAPACITY);
    assert_eq!(device.read(CAPACITY, &mut [0; SECTOR]), Err(Error::Range));
    assert_eq!(device.read(u64::MAX, &mut [0; SECTOR]), Err(Error::Range));
    assert_eq!(device.read(0, &mut []), Err(Error::Size));
    assert_eq!(device.read(0, &mut [0; 513]), Err(Error::Size));
    assert_eq!(device.write(0, &[0; 1024]), Err(Error::Size));
    let mut buffer = [0; SECTOR];
    let phase = match mode {
        BootMode::BlockPersist => {
            device.read(8, &mut buffer).unwrap();
            let fresh = buffer == [0; SECTOR];
            if fresh {
                for sector in [8, 9, CAPACITY - 1] {
                    device.write(sector, &pattern(sector)).unwrap();
                }
                device.flush().unwrap();
            }
            for sector in [8, 9, CAPACITY - 1] {
                device.read(sector, &mut buffer).unwrap();
                assert_eq!(buffer, pattern(sector));
            }
            // Exercise ring slots repeatedly; no assumption that only slot zero is used.
            for _ in 0..20 {
                device.read(9, &mut buffer).unwrap();
                assert_eq!(buffer, pattern(9));
            }
            if fresh { "write" } else { "read" }
        }
        BootMode::BlockReadOnly => {
            assert!(device.geometry().read_only);
            assert_eq!(device.write(8, &pattern(8)), Err(Error::ReadOnly));
            device.read(8, &mut buffer).unwrap();
            assert_eq!(buffer, [0; SECTOR]);
            "readonly"
        }
        BootMode::BlockError => {
            // Test-only bypass of range/opcode admission: real device status is checked.
            assert_eq!(
                device.submit(1, CAPACITY, &mut buffer, true),
                Err(Error::Io)
            );
            assert_eq!(
                device.submit(0xffff, 0, &mut [], true),
                Err(Error::Unsupported)
            );
            device.read(8, &mut buffer).unwrap();
            assert_eq!(buffer, [0; SECTOR]);
            "error"
        }
        BootMode::BlockTimeout => {
            // Publish a real queue request but suppress its notification to the device.
            assert_eq!(device.submit(0, 8, &mut buffer, false), Err(Error::Timeout));
            assert_eq!(device.read(8, &mut buffer), Err(Error::Protocol));
            device.shutdown(memory).unwrap();
            assert_eq!(memory.free_frames(), before);
            device = Device::open(memory).unwrap();
            device.read(8, &mut buffer).unwrap();
            assert_eq!(buffer, [0; SECTOR]);
            "timeout"
        }
        _ => panic!("not a block fixture"),
    };
    device.shutdown(memory).unwrap();
    assert_eq!(memory.free_frames(), before);
    emit(phase, before, memory.free_frames(), dma_frames);
}
fn emit(phase: &str, before: usize, after: usize, frames: usize) {
    let mut serial = Serial::take().unwrap();
    writeln!(serial, "RUSTIC BLOCK verified=1 phase={phase} sectors={CAPACITY} sector_bytes=512 max_bytes=512 dma_frames={frames} rejected={} free_before={before} free_after={after}", if phase == "missing" { 0 } else { 5 }).unwrap();
    serial.flush();
}
