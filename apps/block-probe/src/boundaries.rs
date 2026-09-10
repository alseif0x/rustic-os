// SPDX-License-Identifier: Apache-2.0
use super::{persistence::finish, raw};
use rustic_sdk::{abi::block::*, block::Device};
fn submit(handle: u64, bytes: &[u8]) -> u64 {
    raw::call(SUBMIT, handle, bytes.as_ptr() as u64, bytes.len() as u64)
}
pub(super) fn run(handle: u64, device: &Device) -> u64 {
    let mut count = 0;
    let mut rejected = |actual, error: Error| {
        assert_eq!(actual, error.code());
        count += 1;
    };
    let read = Request {
        operation: Operation::Read,
        sector: 8,
        address: 0,
        length: 512,
    }
    .encode();
    for (offset, value, error) in [
        (0, 2, Error::Version),
        (2, 4, Error::Request),
        (4, 1, Error::Request),
        (28, 1, Error::Request),
        (25, 0, Error::Size),
        (16, 1, Error::Request),
    ] {
        let mut bad = read;
        bad[offset] = value;
        rejected(submit(handle, &bad), error);
    }
    for length in [0, 31, 33, u64::MAX] {
        rejected(
            raw::call(SUBMIT, handle, read.as_ptr() as u64, length),
            Error::Size,
        );
    }
    for address in [
        0,
        0x2000,
        0xffff_8000_0000_0000,
        u64::MAX - 15,
        0x8000_0000 - 16,
    ] {
        rejected(raw::call(SUBMIT, handle, address, 32), Error::Address);
    }
    for address in [
        0,
        0x2000,
        0xffff_8000_0000_0000,
        u64::MAX - 255,
        0x8000_0000 - 256,
    ] {
        let bytes = Request {
            operation: Operation::Write,
            sector: 8,
            address,
            length: 512,
        }
        .encode();
        rejected(submit(handle, &bytes), Error::Address);
    }
    for sector in [8_388_608, u64::MAX] {
        rejected(device.read(sector).unwrap_err().code(), Error::Range);
    }
    let flush = Request {
        operation: Operation::Flush,
        sector: 1,
        address: 0,
        length: 0,
    }
    .encode();
    rejected(submit(handle, &flush), Error::Request);
    let id = device.read(8).unwrap();
    device.wait(id).unwrap();
    // Bad copy-outs must neither write a prefix nor consume the retained completion.
    rejected(
        raw::call(RESULT, handle, raw::cross_page(), 543),
        Error::Size,
    );
    for address in [0, 0x400000, u64::MAX - 255, 0x8000_0000 - 256] {
        rejected(
            raw::call(RESULT, handle, address, RESULT_BYTES as u64),
            Error::Address,
        );
    }
    assert_eq!(
        raw::call(RESULT, handle, raw::cross_page(), RESULT_BYTES as u64),
        RESULT_BYTES as u64
    );
    let result = raw::cross_result();
    assert_eq!(result.id, id);
    assert_eq!(result.data, [0; SECTOR]);
    rejected(device.result().unwrap_err().code(), Error::NoRequest);
    // A valid cross-page input range is copied as bytes, not assumed physically contiguous.
    let write = Request {
        operation: Operation::Write,
        sector: 8,
        address: raw::cross_page() + 32,
        length: 512,
    }
    .encode();
    finish(device, Error::decode(submit(handle, &write)).unwrap());
    finish(device, device.flush().unwrap());
    assert_eq!(finish(device, device.read(8).unwrap()).data, [0; SECTOR]);
    rejected(raw::call(CLOSE, handle, 1, 0), Error::Request);
    assert_eq!(raw::call(CLOSE, handle, 0, 0), 0);
    rejected(device.read(8).unwrap_err().code(), Error::Handle);
    rejected(device.geometry().unwrap_err().code(), Error::Handle);
    count
}
pub(super) fn foreign(handle: u64) -> u64 {
    let device = Device::from_bootstrap(handle);
    assert_eq!(device.geometry(), Err(Error::Handle));
    assert_eq!(device.read(0), Err(Error::Handle));
    assert_eq!(device.result(), Err(Error::Handle));
    assert_eq!(device.cancel(1), Err(Error::Handle));
    assert_eq!(device.close(), Err(Error::Handle));
    5
}
pub(super) fn scoped(device: &Device) -> u64 {
    let geometry = device.geometry().unwrap();
    assert_eq!(geometry.sectors, 2);
    assert_eq!(geometry.rights, READ);
    assert_eq!(device.write(0, &[1; SECTOR]), Err(Error::Denied));
    assert_eq!(device.flush(), Err(Error::Denied));
    assert_eq!(device.read(2), Err(Error::Range));
    assert_eq!(device.read(u64::MAX), Err(Error::Range));
    assert_eq!(finish(device, device.read(0).unwrap()).data, [0; SECTOR]);
    4
}
