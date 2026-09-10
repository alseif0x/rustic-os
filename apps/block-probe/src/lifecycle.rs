// SPDX-License-Identifier: Apache-2.0
use super::persistence::{finish, pattern};
use rustic_sdk::{
    block::{Device, Effect, Error, SECTOR, Status},
    process,
};
pub(super) fn run(handle: u64, role: u64, expected: u64) -> u64 {
    let device = Device::from_bootstrap(handle);
    match role {
        4 => {
            let id = device.read(8).unwrap();
            process::report(id).unwrap();
            loop {
                core::hint::spin_loop();
            }
        }
        5 => {
            let id = device.read(8).unwrap();
            device.wait(id).unwrap();
            let result = device.result().unwrap();
            assert_eq!(result.id, id);
            assert_eq!(result.status as u64, expected);
            assert_eq!(result.effect, Effect::None);
        }
        6 => {
            let mut data = pattern(8);
            let id = device.write(8, &data).unwrap();
            data.fill(0xee);
            process::report(u64::from(data[0])).unwrap();
            finish(&device, id);
            assert_eq!(finish(&device, device.read(8).unwrap()).data, pattern(8));
            finish(&device, device.write(8, &[0; SECTOR]).unwrap());
            finish(&device, device.flush().unwrap());
        }
        7 => {
            assert_eq!(device.read(8), Err(Error::Busy));
        }
        8 => {
            let id = device.write(8, &[0xab; SECTOR]).unwrap();
            assert_eq!(device.cancel(id), Ok(true));
            device.wait(id).unwrap();
            let result = device.result().unwrap();
            assert_eq!(result.status, Status::Cancelled);
            assert_eq!(result.effect, Effect::None);
        }
        9 => {
            let id = device.write(8, &[0xab; SECTOR]).unwrap();
            assert_eq!(device.cancel(id), Ok(false));
            device.wait(id).unwrap();
            let result = device.result().unwrap();
            assert_eq!(result.id, id);
            assert_eq!(result.status, Status::Timeout);
            assert_eq!(result.effect, Effect::Unknown);
        }
        10 => {
            device.read(8).unwrap();
            device.close().unwrap();
            assert_eq!(Device::from_bootstrap(handle).read(8), Err(Error::Handle));
        }
        _ => unreachable!(),
    }
    1
}
