// SPDX-License-Identifier: Apache-2.0
//! Dynamic heap calls. The process record decides the range, the architecture
//! layer only executes it; a mapping failure gives the reservation back.
use super::{Action, Process};
use crate::arch::memory::Memory;
// Qualified on purpose: the `WINDOW_BASE` and `WINDOW_PAGES` query selectors
// carry the same names as the kernel heap policy values they select.
use rustic_abi::memory as abi;
use rustic_abi::runtime::Error;
use rustic_kernel::process::heap;

pub(super) fn dispatch(number: u64, process: &mut Process, memory: &mut Memory) -> Action {
    let [first, second, third] = process.frame.arguments();
    let result = match number {
        abi::MAP => map(process, memory, first, second, third),
        abi::UNMAP => unmap(process, memory, first, second, third),
        abi::QUERY => query(process, first),
        _ => Err(Error::Invalid),
    };
    Action::Return(result.unwrap_or_else(Error::code))
}

/// Region failures keep their meaning in the shared native error encoding.
fn classify(error: heap::Error) -> Error {
    match error {
        heap::Error::Size => Error::Size,
        heap::Error::Address => Error::Address,
        heap::Error::Invalid => Error::Invalid,
        heap::Error::Full => Error::Full,
    }
}

fn map(
    process: &mut Process,
    memory: &mut Memory,
    address: u64,
    pages: u64,
    flags: u64,
) -> Result<u64, Error> {
    if !abi::flags_valid(flags) {
        return Err(Error::Invalid);
    }
    let requested = (address != 0).then_some(address);
    let base = process.heap.reserve(requested, pages).map_err(classify)?;
    match memory.map_user_pages(&mut process.space, base, pages, flags & abi::WRITE != 0) {
        Ok(()) => Ok(base),
        Err(_) => {
            // Frames ran out: nothing stays mapped and nothing stays accounted.
            process
                .heap
                .release(base, pages)
                .expect("release the reservation of this call");
            Err(Error::Full)
        }
    }
}

fn unmap(
    process: &mut Process,
    memory: &mut Memory,
    address: u64,
    pages: u64,
    reserved: u64,
) -> Result<u64, Error> {
    if reserved != 0 {
        return Err(Error::Invalid);
    }
    // The region rejects a partial range before any page is released.
    process.heap.release(address, pages).map_err(classify)?;
    memory
        .unmap_user_pages(&mut process.space, address, pages)
        .expect("the region tracks exactly the mapped heap pages");
    Ok(0)
}

/// Window and limits are policy the guest reads instead of assuming.
fn query(process: &Process, selector: u64) -> Result<u64, Error> {
    match selector {
        abi::MAPPED_PAGES => Ok(process.heap.used()),
        abi::PAGE_LIMIT => Ok(heap::PROCESS_PAGES),
        abi::WINDOW_BASE => Ok(heap::WINDOW_BASE),
        abi::WINDOW_PAGES => Ok(heap::WINDOW_PAGES),
        _ => Err(Error::Invalid),
    }
}
