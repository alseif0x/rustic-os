// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::block::{Error, Geometry, queue::Layout};
#[test]
fn sector_contract_rejects_overflow_sizes_and_readonly() {
    let disk = Geometry {
        sectors: 8_388_608,
        read_only: false,
    };
    assert_eq!(disk.validate(8_388_607, 512, true), Ok(()));
    for sector in [8_388_608, u64::MAX] {
        assert_eq!(disk.validate(sector, 512, false), Err(Error::Range));
    }
    for size in [0, 1, 511, 513, 1024, usize::MAX] {
        assert_eq!(disk.validate(0, size, false), Err(Error::Size));
    }
    let ro = Geometry {
        read_only: true,
        ..disk
    };
    assert_eq!(ro.validate(0, 512, true), Err(Error::ReadOnly));
    assert_eq!(ro.validate(0, 512, false), Ok(()));
}
#[test]
fn queue_bounds_layout_and_counter_wrap() {
    for size in [0, 1, 2, 3, 5, 255, 257, usize::MAX] {
        assert!(Layout::new(size).is_err());
    }
    let small = Layout::new(8).unwrap();
    assert_eq!((small.available, small.used, small.pages), (128, 4096, 2));
    let large = Layout::new(256).unwrap();
    assert_eq!((large.available, large.used, large.pages), (4096, 8192, 3));
    assert_eq!(Layout::completed(u16::MAX, 0, 0, 0), Ok(()));
    assert_eq!(Layout::completed(0, 2, 0, 0), Err(Error::Protocol));
    assert_eq!(Layout::completed(0, 1, 3, 0), Err(Error::Protocol));
    assert_eq!(Layout::completed(0, 1, 0, 255), Err(Error::Protocol));
    assert_eq!(Layout::completed(0, 1, 0, 1), Err(Error::Io));
}
