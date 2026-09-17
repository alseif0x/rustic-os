// SPDX-License-Identifier: Apache-2.0
//! Host tests for the v6 payload extents and free-space accounting (#51).
use rustic_fs::{
    DATA_BYTES_V6, DATA_SECTORS, EXTENTS_PER_FILE, Error, Extent, Extents, FILE_SECTORS_MAX,
    FreeSpace, MAP_WORDS, MAX_FILE_V6,
};

fn map() -> Vec<u64> {
    vec![0; MAP_WORDS]
}

#[test]
fn the_region_and_its_metadata_cost_are_the_selected_budget() {
    // 64 MiB at 512-byte sectors, one bitmap bit per sector: 16 KiB of metadata.
    assert_eq!(DATA_BYTES_V6, 64 * 1024 * 1024);
    assert_eq!(DATA_SECTORS, 131_072);
    assert_eq!(MAP_WORDS, 2048);
    assert_eq!(MAP_WORDS * 8, 16 * 1024);
    assert_eq!(MAX_FILE_V6, 256 * 1024);
    assert_eq!(FILE_SECTORS_MAX, 512);
    assert_eq!(EXTENTS_PER_FILE, 8);
}

#[test]
fn allocation_is_first_fit_and_release_returns_exactly_what_it_took() {
    let mut words = map();
    let mut space = FreeSpace::new(&mut words).unwrap();
    let initial = space.free_sectors();
    assert_eq!(initial, DATA_SECTORS);

    let first = space.allocate(4).unwrap();
    assert_eq!(first, Extent::new(0, 4));
    let second = space.allocate(2).unwrap();
    assert_eq!(second, Extent::new(4, 2));
    assert_eq!(space.free_sectors(), initial - 6);

    space.release(first).unwrap();
    // The released run is the first fit again.
    assert_eq!(space.allocate(4).unwrap(), Extent::new(0, 4));
    assert_eq!(space.free_sectors(), initial - 6);

    for run in [Extent::new(0, 4), second] {
        space.release(run).unwrap();
    }
    assert_eq!(space.free_sectors(), initial);
    assert_eq!(space.free_bytes(), DATA_BYTES_V6);
}

#[test]
fn exhaustion_is_full_and_release_of_unallocated_sectors_is_refused() {
    let mut words = map();
    let mut space = FreeSpace::new(&mut words).unwrap();
    // A full region refuses any request honestly.
    space.fill();
    assert_eq!(space.free_sectors(), 0);
    assert_eq!(space.allocate(1), Err(Error::Full));
    // Freeing a used sector is the normal path; doing it twice is not.
    space.release(Extent::new(0, 1)).unwrap();
    assert_eq!(space.release(Extent::new(0, 1)), Err(Error::Invalid));
    assert_eq!(space.free_sectors(), 1);

    // A later free run serves exactly one request of its size, and the earlier
    // sector is still the first fit for a smaller request.
    let last = DATA_SECTORS - 8;
    space.release(Extent::new(last, 8)).unwrap();
    assert_eq!(space.free_sectors(), 9);
    assert_eq!(space.allocate(8).unwrap(), Extent::new(last, 8));
    assert_eq!(space.free_sectors(), 1);
    assert_eq!(space.allocate(1).unwrap(), Extent::new(0, 1));
    assert_eq!(space.allocate(1), Err(Error::Full));

    // A zero-length or oversized request is a size error, not exhaustion.
    assert_eq!(space.allocate(0), Err(Error::Size));
    assert_eq!(space.allocate(FILE_SECTORS_MAX + 1), Err(Error::Size));
}

#[test]
fn double_release_and_out_of_range_runs_cannot_change_the_accounting() {
    let mut words = map();
    let mut space = FreeSpace::new(&mut words).unwrap();
    let run = space.allocate(3).unwrap();
    space.release(run).unwrap();
    assert_eq!(space.release(run), Err(Error::Invalid));
    assert_eq!(space.free_sectors(), DATA_SECTORS);
    assert_eq!(
        space.release(Extent::new(DATA_SECTORS - 1, 2)),
        Err(Error::Invalid)
    );
    assert_eq!(space.release(Extent::new(0, 0)), Err(Error::Invalid));

    // The map must be exactly one bit per payload sector.
    let mut short = vec![0u64; MAP_WORDS - 1];
    assert_eq!(FreeSpace::new(&mut short).err(), Some(Error::Size));
}

#[test]
fn a_file_may_hold_a_bounded_number_of_runs_and_a_bounded_length() {
    let mut extents = Extents::new();
    assert!(extents.is_empty());
    // Eight runs of 64 sectors are 256 KiB, the file limit, exactly.
    for index in 0..EXTENTS_PER_FILE {
        extents.push(Extent::new(index as u64 * 64, 64)).unwrap();
    }
    assert_eq!(extents.len(), EXTENTS_PER_FILE);
    assert_eq!(extents.sectors(), FILE_SECTORS_MAX);
    assert_eq!(extents.bytes(), MAX_FILE_V6 as u64);
    // A ninth run needs a control-record field that does not exist.
    assert_eq!(
        extents.push(Extent::new(0, 1)),
        Err(Error::Full),
        "the extent list must bound the control record"
    );
    // Offsets follow the order the bytes were written, across runs.
    assert_eq!(extents.offset_of(0), Some(0));
    assert_eq!(extents.offset_of(1), Some(32 * 1024));
    assert_eq!(extents.offset_of(EXTENTS_PER_FILE - 1), Some(224 * 1024));
    assert_eq!(extents.offset_of(EXTENTS_PER_FILE), None);
    assert_eq!(
        extents.runs().map(|run| run.sectors).sum::<u64>(),
        extents.sectors()
    );
}

#[test]
fn a_file_cannot_be_longer_than_its_limit_or_hold_empty_runs() {
    let mut extents = Extents::new();
    assert_eq!(extents.push(Extent::new(0, 0)), Err(Error::Invalid));
    // 512 sectors fit; one more does not.
    extents.push(Extent::new(0, FILE_SECTORS_MAX)).unwrap();
    assert_eq!(extents.push(Extent::new(0, 1)), Err(Error::Size));
}
