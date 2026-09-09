// SPDX-License-Identifier: Apache-2.0
//! Trusted-loader boundary. Limine owns pointer validity/lifetime until entry;
//! semantic map validation below does not sandbox a malicious bootloader.
use limine::{
    BaseRevision,
    memory_map::EntryType,
    request::{
        ExecutableAddressRequest, ExecutableCmdlineRequest, HhdmRequest, MemoryMapRequest,
        RequestsEndMarker, RequestsStartMarker, StackSizeRequest,
    },
};
use rustic_kernel::boot::{BootMode, MapSummary, validate_map};

#[used]
// SAFETY: Linker retains this marker before all request records.
#[unsafe(link_section = ".limine_requests_start")]
static START: RequestsStartMarker = RequestsStartMarker::new();
#[used]
// SAFETY: Loader-visible request section, writable before kernel entry.
#[unsafe(link_section = ".limine_requests")]
static REVISION: BaseRevision = BaseRevision::with_revision(3);
#[used]
// SAFETY: Static typed protocol request retained by the linker.
#[unsafe(link_section = ".limine_requests")]
static MAP: MemoryMapRequest = MemoryMapRequest::new();
#[used]
// SAFETY: Static typed protocol request retained by the linker.
#[unsafe(link_section = ".limine_requests")]
static CMDLINE: ExecutableCmdlineRequest = ExecutableCmdlineRequest::new();
#[used]
// SAFETY: Loader supplies a 64-KiB boot stack before calling the entry point.
#[unsafe(link_section = ".limine_requests")]
static STACK: StackSizeRequest = StackSizeRequest::new().with_size(64 * 1024);
#[used]
// SAFETY: Resident protocol request; only the trusted loader writes its response.
#[unsafe(link_section = ".limine_requests")]
static HHDM: HhdmRequest = HhdmRequest::new();
#[used]
// SAFETY: Resident protocol request supplies the loaded ELF's physical base.
#[unsafe(link_section = ".limine_requests")]
static ADDRESS: ExecutableAddressRequest = ExecutableAddressRequest::new();
#[used]
// SAFETY: Linker retains this marker after all request records.
#[unsafe(link_section = ".limine_requests_end")]
static END: RequestsEndMarker = RequestsEndMarker::new();

pub(super) fn inspect() -> Result<(MapSummary, BootMode), &'static str> {
    if !REVISION.is_supported() {
        return Err("unsupported_base_revision");
    }
    if STACK.get_response().is_none() {
        return Err("missing_stack_response");
    }
    let map = MAP.get_response().ok_or("missing_memory_map")?;
    let summary = validate_map(map.entries().iter().map(|entry| {
        (
            entry.base,
            entry.length,
            entry.entry_type == EntryType::USABLE,
        )
    }))
    .map_err(|_| "invalid_memory_map")?;
    let cmdline = CMDLINE.get_response().ok_or("missing_command_line")?;
    let mode = BootMode::parse(cmdline.cmdline().to_bytes()).ok_or("invalid_boot_mode")?;
    Ok((summary, mode))
}

pub(super) fn memory_layout() -> Result<crate::arch::memory::BootMemory, &'static str> {
    let hhdm = HHDM.get_response().ok_or("missing_hhdm")?;
    let address = ADDRESS.get_response().ok_or("missing_executable_address")?;
    Ok(crate::arch::memory::BootMemory {
        hhdm: hhdm.offset(),
        physical_base: address.physical_base(),
        virtual_base: address.virtual_base(),
    })
}

pub(super) fn memory_regions() -> impl Iterator<Item = (u64, u64, bool)> + Clone {
    MAP.get_response()
        .expect("map already validated")
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.base,
                entry.length,
                entry.entry_type == EntryType::USABLE,
            )
        })
}
