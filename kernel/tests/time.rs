// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::time::{Deadline, Overflow, WaitError, WaitSet, ticks_to_nanos};

#[test]
fn deadlines_never_wrap_or_complete_early() {
    let deadline = Deadline::after(10, 3).unwrap();
    assert!(!deadline.reached(12));
    assert!(deadline.reached(13));
    assert!(deadline.reached(14));
    assert!(Deadline::after(10, 0).unwrap().reached(10));
    assert_eq!(Deadline::after(u64::MAX, 1), Err(Overflow));
}

#[test]
fn simultaneous_waits_complete_once_and_cancel_independently() {
    let mut waits = WaitSet::<4>::new();
    for (slot, delay) in [5, 2, 2, 9].into_iter().enumerate() {
        waits
            .register(slot, Deadline::after(10, delay).unwrap())
            .unwrap();
    }
    assert_eq!(
        waits.register(0, Deadline::after(10, 1).unwrap()),
        Err(WaitError::Occupied)
    );
    assert_eq!(waits.cancel(4), Err(WaitError::InvalidSlot));
    assert_eq!(waits.cancel(3), Ok(true));
    assert_eq!(waits.cancel(3), Ok(false));
    assert_eq!(waits.next_deadline().unwrap().ticks(), 12);
    assert_eq!(waits.complete(11), [false; 4]);
    assert_eq!(waits.complete(12), [false, true, true, false]);
    assert_eq!(waits.complete(12), [false; 4]);
    assert_eq!(waits.complete(100), [true, false, false, false]);
    assert_eq!(waits.next_deadline(), None);
}

#[test]
fn rational_clock_conversion_is_bounded() {
    assert_eq!(ticks_to_nanos(100, 11932, 1193182), Some(1_000_015_085));
    assert_eq!(ticks_to_nanos(u64::MAX, 11932, 1193182), Some(u64::MAX));
    assert_eq!(ticks_to_nanos(1, 0, 1), None);
    assert_eq!(ticks_to_nanos(1, 1, 0), None);
}
