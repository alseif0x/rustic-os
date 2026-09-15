// SPDX-License-Identifier: Apache-2.0
//! Guest evidence for bounded dynamic memory, executed in ring 3.
//!
//! The phase queries the kernel's limits, reserves one page, reuses a released
//! block, grows the run and touches every byte of freshly mapped pages, shrinks
//! it again, is refused a growth beyond the per-process limit and keeps working,
//! is refused an unmap of a run it does not own, and finally releases
//! everything. The result is packed into one diagnostic report word because the
//! trusted fixture retains only the last integer a process reports.
//!
//! Layout of that word, decoded by `kernel/src/process/runtime/tests/sdk.rs`:
//!
//! | Bits | Meaning |
//! | --- | --- |
//! | 63:56 | Tag `0x48` |
//! | 55:48 | Per-process page limit read from the kernel |
//! | 47:40 | Peak pages this heap held at once |
//! | 39:32 | Pages still mapped after the heap was dropped |
//! | 31:12 | Peak bytes committed inside the region |
//! | 11:4 | Reserved, zero |
//! | 3 | An unmap of a run the process does not own was refused |
//! | 2 | Freshly mapped pages read as zero and were writable |
//! | 1 | A released block satisfied the identical request again |
//! | 0 | Growing past the per-process limit returned `Full` |
use rustic_sdk::abi::runtime;
use rustic_sdk::memory::{
    Error, Heap, PAGE,
    raw::{self, Query},
};

pub(super) const TAG: u64 = 0x48;
const FULL: u64 = 1;
const REUSE: u64 = 1 << 1;
const ZEROED: u64 = 1 << 2;
const GUARDED: u64 = 1 << 3;
/// Repeating byte pattern written over every fresh page.
const PATTERN: usize = 251;

fn protocol<T>() -> Result<T, Error> {
    Err(Error::Map(runtime::Error::Protocol))
}

pub(super) fn run() -> Result<u64, Error> {
    let limit = raw::query(Query::PageLimit)?;
    let window_pages = raw::query(Query::WindowPages)?;
    let window_base = raw::query(Query::WindowBase)?;
    if window_base == 0
        || limit == 0
        || limit > window_pages
        || raw::query(Query::MappedPages)? != 0
    {
        return protocol();
    }
    let mut flags = 0;
    let peak_pages;
    let peak_bytes;
    {
        let mut heap = Heap::reserve(1, true)?;
        if raw::query(Query::MappedPages)? != heap.pages() || heap.base() < window_base {
            return protocol();
        }

        // A small working set with mixed alignments, then the reuse check: the
        // block just released must satisfy the identical request again.
        let first = heap.alloc(24, 1)?;
        let second = heap.alloc(40, 8)?;
        let third = heap.alloc(56, 16)?;
        let fourth = heap.alloc(72, 64)?;
        let reused = third.address();
        heap.free(third)?;
        let third = heap.alloc(56, 16)?;
        if third.address() == reused {
            flags |= REUSE;
        }

        // Growth never relocates, so the new pages start at the current end.
        let mapped = heap.pages();
        heap.grow(3)?;
        peak_pages = heap.pages();
        if raw::query(Query::MappedPages)? != peak_pages {
            return protocol();
        }
        let fresh = heap.base() + mapped * PAGE;
        let wide = heap.alloc(2 * PAGE as usize + 512, 8)?;
        let address = wide.address() as u64;
        if address >= fresh {
            return protocol();
        }
        let offset = (fresh - address) as usize;
        let zeroed = heap
            .bytes(&wide)?
            .get(offset..)
            .is_some_and(|tail| !tail.is_empty() && tail.iter().all(|byte| *byte == 0));
        for (index, byte) in heap.bytes_mut(&wide)?.iter_mut().enumerate() {
            *byte = (index % PATTERN) as u8;
        }
        let written = heap
            .bytes(&wide)?
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte == (index % PATTERN) as u8);
        if !written {
            return protocol();
        }
        if zeroed {
            flags |= ZEROED;
        }
        peak_bytes = heap.stats()?.peak_used;

        // Releasing the wide block frees whole trailing pages again.
        heap.free(wide)?;
        let released = heap.shrink_trailing()?;
        if released == 0
            || heap.pages() + released != peak_pages
            || raw::query(Query::MappedPages)? != heap.pages()
        {
            return protocol();
        }

        // The per-process limit refuses the growth without disturbing the heap.
        match heap.grow(limit) {
            Err(Error::Map(runtime::Error::Full)) => flags |= FULL,
            Err(other) => return Err(other),
            Ok(()) => return protocol(),
        }
        if raw::query(Query::MappedPages)? != heap.pages() {
            return protocol();
        }
        let recovered = heap.alloc(96, 32)?;
        heap.free(recovered)?;

        // A run this process does not own cannot be released: the last window
        // page is unmapped, and the page after the window is out of the window.
        let unowned = raw::unmap(window_base + (window_pages - 1) * PAGE, 1);
        let outside = raw::unmap(window_base + window_pages * PAGE, 1);
        if refused(unowned) && refused(outside) {
            flags |= GUARDED;
        }

        for block in [fourth, third, second, first] {
            heap.free(block)?;
        }
        let stats = heap.stats()?;
        if stats.blocks != 0 || stats.used != 0 || stats.free != stats.capacity {
            return protocol();
        }
    }
    encode(
        limit,
        peak_pages,
        raw::query(Query::MappedPages)?,
        peak_bytes,
        flags,
    )
}

/// Both refusals are acceptable evidence: the address is either not mapped by
/// this process or not inside the heap window at all.
fn refused(result: Result<(), runtime::Error>) -> bool {
    matches!(
        result,
        Err(runtime::Error::Invalid) | Err(runtime::Error::Address)
    )
}

fn encode(
    limit: u64,
    peak_pages: u64,
    final_pages: u64,
    peak_bytes: usize,
    flags: u64,
) -> Result<u64, Error> {
    let bytes = peak_bytes as u64;
    if limit > 0xff || peak_pages > 0xff || final_pages > 0xff || bytes >= 1 << 20 {
        return protocol();
    }
    Ok((TAG << 56)
        | (limit << 48)
        | (peak_pages << 40)
        | (final_pages << 32)
        | (bytes << 12)
        | flags)
}
