<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native block access, version 1

Implemented for #44, 2026-09-10. A separately linked native Rust program runs these calls in ring 3, including disk persistence across two VM boots. This is the storage prerequisite for #12; it does not implement a filesystem or a file service.

## Selected boundary

Use copied, bounded asynchronous requests. The kernel copies an entire write sector before returning from submission, owns the DMA request, and later copies a retained completion to the caller. A read submission has no output pointer: the caller supplies its buffer only when collecting the result. The scheduler polls once per turn outside interrupt context and can run other processes while the device works. Existing synchronous driver tests use a wrapper over the same submission/completion mechanics.

Compared with chunked 64-byte IPC, a 512-byte copied sector avoids at least eight data messages plus framing and assembly state. Compared with waiting inside the syscall, separate submission/completion permits process progress, death and queued cancellation without retaining user memory. No shared memory, user DMA, filesystem logic or JSON is introduced. The cost is two owned request slots and explicit completion collection; one device request remains in flight.

Windows documents that requesting I/O cancellation does not establish its final result; Linux's DMA guide requires respecting device buffer ownership. macOS Dispatch I/O is an asynchronous API reference rather than a runtime available here. RusticOS adapts those lifecycle principles using its own scheduler and handles; it does not implement their ABIs. Sources: [Windows cancellation](https://learn.microsoft.com/en-us/windows/win32/fileio/canceling-pending-i-o-operations), [Linux DMA](https://docs.kernel.org/core-api/dma-api-howto.html), [Apple DispatchIO](https://developer.apple.com/documentation/dispatch/dispatchio), [VirtIO 1.2](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html).

## ABI and ownership

INT 0x80 retains the existing register convention: RAX number/result, RDI/RSI/RDX arguments. Calls 9–14 are an additive extension; process ABI and IPC versions do not change. Block wire version is 1. All integers are explicitly encoded little endian, all reserved bytes are zero, and exact sizes are required. No Rust struct layout crosses the boundary.

| Call | Arguments | Result |
| --- | --- | --- |
| 9 INFO | handle, output pointer, 32 | 32 bytes of scoped geometry; all-zero arguments query version 1 only |
| 10 SUBMIT | handle, request pointer, 32 | Monotonic request ID after complete snapshot/queue admission |
| 11 RESULT | handle, output pointer, 544 | Complete initialized result; consume only after successful full copy-out |
| 12 WAIT | handle, request ID, 0 | 0 when completion exists; otherwise block this process |
| 13 CANCEL | handle, request ID, 0 | 0 cancelled before device submission, 1 too late; query actual completion |
| 14 CLOSE | handle, 0, 0 | 0; invalidate handle immediately, discard queued/completed work and retain active DMA ownership until settlement |

The 32-byte request contains version u16, operation u16 (read=1/write=2/flush=3), reserved u32, relative sector u64, input pointer u64, length u32, reserved u32. Read uses pointer 0/length 512; write uses a valid input range/length 512; flush requires sector/pointer/length all zero. Sector addresses are relative to the granted span. Full range/overflow/rights checks precede queue mutation.

Geometry contains version u16, size u16, granted rights u32, sector count u64, sector size u32, maximum bytes u32, physical read-only flag u32 and reserved u32. Rights are read=1, write=2, flush=4. A flush grant must cover the entire device because this transport has only whole-device flush. The trusted launcher grants at most one handle per live process, at most four total; names and manifest bits grant nothing. Block handles have a separate type domain from IPC tokens while reusing the same owner/rights/monotonic-token mechanism. No application grant/transfer/open-by-name call is added.

A result contains version u16, operation u16, status u32, request ID u64, data length u32, effect u32, reserved u64, then 512 initialized bytes. Read success returns 512 bytes; other results return zero bytes with a zero-filled tail. Status distinguishes success, cancelled, device error, timeout, protocol failure and unavailable. Effect is none, completed or unknown: submitted write/flush failure can be unknown, and neither completion nor cancellation implies durable filesystem transactions. Only successful flush provides the existing driver persistence boundary.

Two fixed request records cover queued, active and completed work together, with one outstanding request per owner. An uncollected completion consumes one slot; a client cannot consume both. Full capacity yields Busy before effect. FIFO admission selects the oldest queued request. No unbounded wait/data queue is added. The existing four process slots suffice for a designated server, two negative clients and a control survivor.

## Lifetime and failure rules

Invalid result pointers/sizes leave completion intact. No pointer survives submission or collection; user address spaces can be reaped while abandoned DMA uses only kernel frames. Closing/exiting cancels queued work without effect and abandons active work without pretending to undo it. Cancelling queued work retains a cancelled completion; cancelling active/completed work returns too late and preserves the actual result.

Each active device request has the existing 25-tick deadline and 5,000,000-poll fallback. The deadline begins at device submission; completed results remain until collection/close/exit. Timeout/protocol failure fences reuse; reset must be read back before DMA frames can be freed. The service may reopen the same fixed device only after reset-confirmed shutdown and matching geometry. A failed reset quarantines the DMA allocation and PCI claim. A restarted process needs a fresh owner-bound handle; old handles and requests never become authority for it. Owner control can execute while I/O is pending; this is bounded polling on one CPU, not an interrupt-driven or SMP storage stack.

## Implementation and review

| Boundary | Modules |
| --- | --- |
| Independent binary codecs | `crates/abi/src/block/`: request, geometry, completion, errors and private wire helpers |
| Shared handle mechanism | `kernel/src/handles/`: owner, rights, per-owner quota, disjoint type domains and monotonic identity |
| Pure admission and lifetime | `kernel/src/block/access/`: grants and bounded request records |
| Hardware | `kernel/src/drivers/block/`: device, publication, bounded poll and synchronous acceptance wrapper |
| Process integration | `process/runtime/block/`, `syscall/block.rs` and `waiters.rs`: owned service, validated copies and typed wakeup |
| Native client and tests | `crates/sdk/src/block/`, `apps/block-probe/` and `process/runtime/tests/block_access/` |
| Independent evaluation | `tools/boot_support/block_user_evidence.py` and the existing disposable-disk oracle |

The implementing agent reviewed dependency direction, visibility, quota/lifetime transitions and every new unsafe boundary; no independent audit is claimed. Shared contracts forbid unsafe code. Hardware volatile access/reset remains within the existing PCI/DMA ownership modules. The SDK reuses its instruction boundary; the probe isolates deliberately invalid integer addresses in its raw-call test module. No user pointer is stored in the broker, driver or blocked-process record. No external crate, source template or package is added.

Status wire values are success=0, cancelled=1, I/O=2, timeout=3, protocol=4, unavailable=5; effects are none=0, completed=1, unknown=2. Admission errors use `u64::MAX - 32 - index`, with indices 0–13 in this fixed order: Handle, Denied, Address, Size, Version, Request, Busy, Quota, Range, ReadOnly, Unavailable, NoRequest, WouldBlock, Protocol. They are separate from completion status and the existing IPC errors. Request IDs stay below 2^56; neither IDs nor handle counters wrap/reuse in a boot. Block tokens use type domain 1 in the upper byte; IPC retains domain 0. Tokens are opaque, not cryptographic credentials.

## Reproduce acceptance

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode block-user --timeout 45
python3 tools/boot.py run --mode block-user-faults --timeout 45
python3 tools/boot.py test --timeout 45
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

Use the pinned R0 Ubuntu/WSL2, Rust 1.98.1 and QEMU configuration in [BLOCK.md](BLOCK.md). The full regression matrix contains **46 Rust tests, 30 Python runner tests, 20 direct VM scenarios and 24 isolated executor scenarios**. The separately maintained JSON service-contract/model checks are not guest evidence.

- `block-user`: one native server plus a preemptible control process. Write/read/flush sectors 8, 9 and 8,388,607; repeat reads through ring reuse; protect sector 0. Terminate QEMU and start another VM on the same disposable disk. Both the native server and the independent host oracle verify the prior data. The two VM phases are mandatory.
- `block-user-faults`: 14 native application executions, 53 counted rejections and six lifecycle scenarios. Reject malformed requests, lengths, address overflow, absent/kernel/readonly output mappings, crossing into absent pages, foreign/stale handles, absent provisioning, scope violations and full queue. Valid cross-page input/output succeeds. Invalid collection preserves the completion. A queued write retains its original snapshot after the caller changes its buffer.
- Lifecycle coverage includes a real device error, notification-withheld timeout, active cancel returning too late, close while active, queued cancellation without submission, and owner death while queued pressure exists. Reap and reuse the dead process's user frames while DMA is still active; reset/reopen the device, complete the queued writer and reject old authority. The host checks the selected disk sectors after the negative sequence.
- The control process advances under pressure. All process/handle/request/DMA allocations are reclaimed after settled shutdown; existing IPC and driver cases remain required. The timeout injection is controlled notification suppression, not a physical unplug or a proof about malicious devices.

Measured direct fault sample: three DMA frames (12,288 bytes), peak 79 simultaneously allocated frames (323,584 bytes, including test processes/page tables and DMA), and 5,232 bytes for the complete manager's fixed metadata. There were 245 control preemptions and free frames returned from 52,644 to 52,644. These are one configuration's accounting/progress observations, not latency/throughput budgets; #20 owns repeated measurements and thresholds. The historical #44 topology used four processes and four block handles, with two outstanding records/one active device request and unchanged IPC capacity.

Direct `image.json` records both applications' ELF/manifest hashes. Isolated jobs export each ELF (at most 1 MiB), each manifest (128 bytes), kernel/image hashes and the existing 2048-byte selected-sector artifact; they build from an exact Git revision. [#44](https://github.com/alseif0x/rustic-os/issues/44) records the accepted revision and CI evidence.

## Remaining boundaries

#12 remains responsible for filesystem format, workspace objects, file-client encoding and crash consistency; #13 supplies product supervision/current-session grants. #44 uses explicit trusted launch fixtures under ADR-0002. The driver still targets one trusted fixed legacy VirtIO device, one CPU and copied sectors. There is no general device discovery, interrupt-driven storage, shared DMA, arbitrary block open, filesystem transaction, power-loss guarantee or complete user session.

The scheduler owner must continue calling `Manager::step` while I/O is outstanding, including when no process is currently ready. `None` means no user process ran that turn; it does not mean asynchronous work is finished. The [native session runtime](NATIVE-RUNTIME.md) now supplies an interrupt-driven idle loop and the terminal supervisor; broader product supervision remains #13. Two deliberately retained completions can occupy all request slots; per-owner quotas bound memory, not availability against every privileged server. Only designated storage servers should receive block grants, and #20/#13 must assess control reservation for the eventual service topology.

