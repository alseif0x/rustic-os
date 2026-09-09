// SPDX-License-Identifier: Apache-2.0
//! Trusted-loader boundary. Limine owns pointer validity/lifetime until entry;
//! semantic map validation below does not sandbox a malicious bootloader.
use limine::{
    BaseRevision,
    memory_map::EntryType,
    request::{
        ExecutableCmdlineRequest, MemoryMapRequest, RequestsEndMarker, RequestsStartMarker,
        StackSizeRequest,
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
