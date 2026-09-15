<!-- SPDX-License-Identifier: Apache-2.0 -->

# Current work state

Updated 2026-09-15. Replace this checkpoint rather than appending conversation history.

## Direction and current increment

Own modular Rust OS; isolated native applications, human and agent clients sharing authority and semantics. MCP is optional. Multiple preempted processes currently execute on one CPU; SMP, independent application installation and physical-machine support remain unimplemented. [Issue #1](https://github.com/alseif0x/rustic-os/issues/1) and the [systems roadmap](architecture/systems-roadmap.md) own sequencing.

Current increment over `4458b73` (uncommitted working tree on `main`): **dynamic user memory with a bounded native SDK allocator** for [#49](https://github.com/alseif0x/rustic-os/issues/49). Three ordinary process syscalls in `crates/abi/src/memory.rs` (`MAP` 21, `UNMAP` 22, `QUERY` 23) map zeroed, never-executable pages inside a per-process heap window (`0x1000_0000`, 128 pages, 64-page budget) whose reservation lives in the pure `kernel/src/process/heap.rs` bitmap; `map_user_pages`/`unmap_user_pages` sit over the existing zeroed mapping primitives with multi-page rollback, ELF segments may not reach the window, reap reclaims through the existing `destroy_user`, and `INFO` word 7 sums heap pages of unreaped processes (not yet forwarded by the supervisor or shell). The SDK adds `memory::{raw, allocator, Heap}`: typed syscalls, a first-fit allocator with in-region three-word headers, coalescing, generation-tagged handles and exact accounting (`Region` is its only `unsafe`), and a `Heap` that grows and shrinks by whole pages; no `GlobalAlloc`. The consumer is `apps/sdk-probe` under the kernel `ok` fixture. [Process ABI](PROCESS-ABI.md), [Memory](MEMORY.md) and [SDK](SDK.md) document the contract and limits.

Previous increment `4458b73`: the second native semantic tasks client (`apps/utility` under `TASKS_OWNER` over `crates/tasks-client`, two-scope grants, deterministic cuts) with its guest suites `tools/tasks_owner_test.py`, the `tasks.owner` phase of `tools/terminal_test.py` and `tools/tasks_owner_normal_test.py`; see [Tasks](TASKS.md) and [Authority](AUTHORITY.md).

## Verified evidence (final tree)

`cargo xtask check` passed: 367 host tests, fmt, Clippy `-D warnings`, kernel builds and kernel-target clippy for `boot-image` and `sdk-test`, guest clippy for every app in `native` and shell/supervisor/utility in `native,tasks-acceptance`. New host tests: `crates/abi/tests/memory.rs` (3), `kernel/tests/process_heap.rs` (6), `kernel/tests/process_elf.rs` window rejection, `crates/sdk/tests/memory.rs` (12: alignment, reuse, coalescing, fragmentation vs empty, stale/foreign/double free, truncate/extend), `tools/tests/test_application.py` golden record.

`python3 tools/boot.py run --mode ok` (build `e6b7fbd87b9c419b`, image SHA-256 `c55f2956e5a86e18fe98f8d027c0d2e448e3d8019ba900d9c56643ff2f17c774`): `RUSTIC MEMORY … rollback=1 zero_reuse=1 …` including the new `map_user_pages` rollback case that drains frames to one short of the measured cost; `RUSTIC SDK … heap_limit=64 heap_peak_pages=4 heap_peak_bytes=9016 heap_full=1 heap_reuse=1 heap_zeroed=1 heap_guarded=1 heap_final_pages=0 free_before=52255 free_after=52255`. Evidence `artifacts/boot/ok/{serial.log,result.json}`.

`python3 tools/terminal_test.py` (build `12c6b66721d25fb1`, kernel SHA-256 `04beffa72db4822445dd21bbaa9c022c76ed071ad81738d53450c6f9bfc04c22`): 2,705 phase-one commands, two boots, all cases including `tasks.owner`. `--mode block-user` and `--mode recovery-test` (49 cases, 98 boots) passed on the same sources.

One Fable low review of the actual diff found no blocker; its two should-fix findings (allocator `extend` growing the region before a fallible header write, stale handles accepted after address reuse) were fixed with tests, and the untested kernel rollback gained a guest case. Remote CI and a fresh isolated-container run are not claimed. The host WSL session dropped or ran out of memory several times; every reported result comes from a completed run.

## Next acceptance

Keep [#49](https://github.com/alseif0x/rustic-os/issues/49) open. The ABI, kernel mapping, SDK allocator and probe evidence are in; still missing are a product application consumer (a `crates/tasks-client` or `apps/utility` buffer moved onto the SDK heap without breaking the #22 byte-for-byte parity), exhaustion evidence taken through the shell control path rather than the kernel fixture, and forwarding `INFO` word 7 through the supervisor and the shell `mem` line (append after `pending_io`; `tools/terminal_support/latency/evidence.py` full-matches that line and must follow). A general kernel heap remains a separate consumer-driven decision.

Known limits to carry: `validate_buffer` caps one user buffer at 4096 bytes, so a heap-backed buffer larger than a page still needs #50; the validate-then-copy and reserve-then-map sequences are safe only because one dispatch performs one action on one CPU (#53); fragmentation with automatic placement reports `Address`; an over-budget request reports `Full` before availability is consulted. From #22: a utility child that takes cut 2 or 3 loses its file binding and needs a successor; `Intent::request` refuses a journal version not above the target's only after retention.

Keep [#22](https://github.com/alseif0x/rustic-os/issues/22) open for general discovery and complete mission acceptance. Keep #47/#43 correctness work finite. Optional file-service semantic/profile expansion remains frozen. #48, #50, #51 and #52 own physical inventory, bulk data, workspace capacity and independent delivery. Review #53 concurrency ownership before expanding runtime assumptions. #54/#37 and #55 remain bounded probes; #56 research does not change the adopted kernel model.

## Workspace constraints

Never experiment on `artifacts/terminal/data.raw`. Preserve the existing owner modification of `LICENSE`; do not stage it incidentally. Commits are authored as the owner (`alseif0x`) with no AI attribution. Maintained docs remain English. Continue using the project orchestration skill: Opus high bounded workers, Fable low review, at most two concurrent children and one owner for shared build/VM tests. On this Windows host every build and QEMU run goes through WSL Ubuntu; the terminal suite's host-side transcript write can fail with `ENOMEM` on the Windows mount and is retried.
