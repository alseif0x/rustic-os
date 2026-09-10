// SPDX-License-Identifier: Apache-2.0
use rustic_sdk::block::{Completion, Device, Effect, SECTOR, Status};
pub(super) const CAPACITY: u64 = 8_388_608;
pub(super) fn pattern(sector: u64) -> [u8; SECTOR] {
    core::array::from_fn(|i| ((sector.wrapping_mul(17) + i as u64 * 29) % 251 + 1) as u8)
}
pub(super) fn finish(device: &Device, id: u64) -> Completion {
    device.wait(id).unwrap();
    let result = device.result().unwrap();
    assert_eq!(result.id, id);
    assert_eq!(result.status, Status::Success);
    result
}
pub(super) fn run(device: &Device) -> u64 {
    assert_eq!(device.geometry().unwrap().sectors, CAPACITY);
    let fresh = finish(device, device.read(8).unwrap()).data == [0; SECTOR];
    if fresh {
        for sector in [8, 9, CAPACITY - 1] {
            assert_eq!(
                finish(device, device.write(sector, &pattern(sector)).unwrap()).effect,
                Effect::Completed
            );
        }
        finish(device, device.flush().unwrap());
    }
    for sector in [8, 9, CAPACITY - 1] {
        assert_eq!(
            finish(device, device.read(sector).unwrap()).data,
            pattern(sector)
        );
    }
    for _ in 0..20 {
        assert_eq!(finish(device, device.read(9).unwrap()).data, pattern(9));
    }
    assert_eq!(finish(device, device.read(0).unwrap()).data, [0; SECTOR]);
    if fresh { 1 } else { 2 }
}
