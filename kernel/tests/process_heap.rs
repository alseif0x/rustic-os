// SPDX-License-Identifier: Apache-2.0
use rustic_kernel::memory::PAGE_SIZE;
use rustic_kernel::process::heap::{
    Error, PROCESS_PAGES, Region, WINDOW_BASE, WINDOW_END, WINDOW_PAGES,
};

fn page(index: u64) -> u64 {
    WINDOW_BASE + index * PAGE_SIZE
}

#[test]
fn automatic_placement_takes_the_lowest_run_that_fits() {
    let mut region = Region::new();
    assert_eq!(region.reserve(None, 1), Ok(page(0)));
    assert_eq!(region.reserve(None, 4), Ok(page(1)));
    assert_eq!(region.reserve(None, 2), Ok(page(5)));
    assert_eq!(region.used(), 7);
    // Freeing the middle run leaves a hole the next request reuses first.
    assert_eq!(region.release(page(1), 4), Ok(()));
    assert_eq!(region.used(), 3);
    assert_eq!(region.reserve(None, 2), Ok(page(1)));
    assert_eq!(region.reserve(None, 3), Ok(page(7)));
    assert_eq!(region.reserve(None, 2), Ok(page(3)));
    assert_eq!(region.used(), 10);
}

#[test]
fn fragmentation_refuses_a_run_that_does_not_fit_contiguously() {
    let mut region = Region::new();
    // Own every third page: free runs are never longer than two pages.
    let owned = (0..WINDOW_PAGES).step_by(3).count() as u64;
    for index in (0..WINDOW_PAGES).step_by(3) {
        assert_eq!(region.reserve(Some(page(index)), 1), Ok(page(index)));
    }
    assert_eq!(region.used(), owned);
    assert!(owned < PROCESS_PAGES, "budget must remain available");
    // Budget and free pages remain, yet no run of three pages exists.
    assert_eq!(region.reserve(None, 3), Err(Error::Address));
    assert_eq!(region.reserve(Some(page(1)), 3), Err(Error::Address));
    assert_eq!(region.reserve(None, 2), Ok(page(1)));
    assert_eq!(region.release(page(3), 1), Ok(()));
    // The released page joins its free neighbours into a run of three.
    assert_eq!(region.reserve(None, 3), Ok(page(3)));
    assert_eq!(region.used(), owned + 4);
}

#[test]
fn explicit_addresses_must_be_aligned_inside_the_window_and_free() {
    let mut region = Region::new();
    assert_eq!(region.reserve(Some(page(8)), 4), Ok(page(8)));
    for (address, pages) in [
        (page(8), 1),
        (page(11), 1),
        (page(7), 2),
        (page(6), 8),
        (page(8), 4),
    ] {
        assert_eq!(region.reserve(Some(address), pages), Err(Error::Address));
    }
    for address in [
        0,
        PAGE_SIZE,
        WINDOW_BASE - PAGE_SIZE,
        WINDOW_BASE + 1,
        page(1) + 8,
        WINDOW_END,
        u64::MAX - (PAGE_SIZE - 1),
    ] {
        assert_eq!(region.reserve(Some(address), 1), Err(Error::Address));
    }
    // A range leaving the window is refused even when its start is valid.
    assert_eq!(
        region.reserve(Some(page(WINDOW_PAGES - 2)), 4),
        Err(Error::Address)
    );
    assert_eq!(region.used(), 4);
    assert_eq!(region.reserve(Some(page(12)), 4), Ok(page(12)));
    assert!(region.owns(page(8), 8));
}

#[test]
fn partial_and_repeated_releases_change_nothing() {
    let mut region = Region::new();
    assert_eq!(region.reserve(Some(page(4)), 4), Ok(page(4)));
    assert_eq!(region.release(page(2), 4), Err(Error::Invalid));
    assert_eq!(region.release(page(6), 4), Err(Error::Invalid));
    assert_eq!(region.release(page(4), 8), Err(Error::Invalid));
    assert_eq!(region.release(page(4), 0), Err(Error::Size));
    assert_eq!(region.release(page(4) + 1, 1), Err(Error::Address));
    assert_eq!(region.release(WINDOW_END, 1), Err(Error::Address));
    assert_eq!(region.used(), 4);
    assert!(region.owns(page(4), 4));
    // Releasing part of an owned run is allowed; the rest stays owned.
    assert_eq!(region.release(page(4), 2), Ok(()));
    assert_eq!(region.release(page(4), 2), Err(Error::Invalid));
    assert_eq!(region.release(page(6), 2), Ok(()));
    assert_eq!(region.release(page(6), 2), Err(Error::Invalid));
    assert_eq!(region.used(), 0);
    assert!(!region.owns(page(4), 1));
}

#[test]
fn the_per_process_limit_bounds_every_request_and_accounting_returns_to_zero() {
    let mut region = Region::new();
    assert_eq!(region.reserve(None, 0), Err(Error::Size));
    assert_eq!(region.reserve(None, PROCESS_PAGES + 1), Err(Error::Size));
    assert_eq!(region.reserve(None, WINDOW_PAGES), Err(Error::Size));
    assert_eq!(region.reserve(Some(page(0)), u64::MAX), Err(Error::Size));
    assert_eq!(region.reserve(None, PROCESS_PAGES), Ok(page(0)));
    assert_eq!(region.used(), PROCESS_PAGES);
    assert_eq!(region.reserve(None, 1), Err(Error::Full));
    assert_eq!(
        region.reserve(Some(page(PROCESS_PAGES)), 1),
        Err(Error::Full)
    );
    assert_eq!(region.release(page(0), PROCESS_PAGES), Ok(()));
    assert_eq!(region.used(), 0);
    // The whole budget is available again after the release.
    assert_eq!(region.reserve(None, PROCESS_PAGES), Ok(page(0)));
    assert_eq!(region.used(), PROCESS_PAGES);
}

#[test]
fn the_window_stays_between_the_image_and_the_stack_guard() {
    use rustic_kernel::process::elf::{STACK_GUARD, STACK_TOP};
    assert_eq!(WINDOW_BASE, 0x1000_0000);
    assert_eq!(WINDOW_END, WINDOW_BASE + WINDOW_PAGES * PAGE_SIZE);
    // The policy constants are checked at compile time as well as here.
    const { assert!(PROCESS_PAGES < WINDOW_PAGES) };
    const { assert!(WINDOW_BASE > 0x40_0000 && WINDOW_END < STACK_GUARD) };
    const { assert!(STACK_GUARD < STACK_TOP) };
}
