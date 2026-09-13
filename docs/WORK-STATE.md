<!-- SPDX-License-Identifier: Apache-2.0 -->

# Current work state

Updated 2026-09-13. Replace this checkpoint rather than appending conversation history.

## Direction and current increment

Own modular Rust OS; isolated native applications, human and agent clients sharing authority and semantics. MCP is optional. Multiple preempted processes currently execute on one CPU; SMP, independent application installation and physical-machine support remain unimplemented. [Issue #1](https://github.com/alseif0x/rustic-os/issues/1) and the [systems roadmap](architecture/systems-roadmap.md) own sequencing.

Current increment over `72dbc8c`: **ordinary `tasks add PATH TITLE` and `tasks done PATH ID` now apply immutable native plans**. Listing and read-only previews remain available. `tasks enable` explicitly activates existing persistent format 3; no new file-service profile is introduced. [Tasks](TASKS.md) describes usage, recovery and bounds.

The tasks ELF owns document parsing and edit planning and retains READ-only scope. Its independent contract crate carries canonical bounded chunks; the supervisor owns asynchronous collection, authorization and result cleanup. The shell's owner client owns a separate intent codec, journal, mutation/recovery flow and presentation. No kernel, SDK, unsafe, heap or quota expansion. The shell directly reuses the already reviewed SHA-256 workspace dependency.

## Retention and recovery boundary

- Reserve `/config/tasks-intent` for one owner-controlled immutable record. Atomic ordinary replacement and exact pinned readback precede target submission. The committed journal version supplies the durable initial retry key under owner subject `1`.
- Apply through existing `replace_file`. Require exact receipt correspondence, receipt version greater than the journal key, then pinned target bytes before version-checked journal cleanup. An empty journal represents idle state; it occupies one file slot.
- One unresolved intent blocks new mutations. `tasks recover` only queries the original identity; it never rebases or resubmits. `restart files` supplies a fresh owner binding after disconnection and revokes utility sessions.
- Lost initial journal acknowledgement, absent/expired outcomes, collisions and later human changes preserve explicit uncertainty. `tasks forget KEY` deliberately discards recovery evidence and does not cancel or undo an effect. The journal is not protected against its owner erasing or rolling back storage.
- Two retained outcomes remain a real global limit. Tasks report `Full`; they do not rotate receipts automatically. Already-done is an exact no-op without journal or receipt writes.

This resolves the previous initial-key gate using application-owned durable state rather than a new admission profile or a post-commit service instance. It does not complete the general catalog or M1.

## Next acceptance

Keep [#22](https://github.com/alseif0x/rustic-os/issues/22) open. Establish a second native semantic client's parity and its scoped/revoked recovery evidence using this concrete consumer. Share the client mechanism at a real reuse boundary; preserve app-owned planning, exact intent and owner authority. Do not treat the shell's failure-cut fixture as a separately implemented semantic client. General discovery and complete mission acceptance remain open.

Keep #47/#43 correctness work finite. Optional file-service semantic/profile expansion remains frozen without a demonstrated consumer need or defect. #48–#52 own memory/allocation, bulk data, workspace capacity and independent delivery; this demo does not replace them. Review #53 concurrency ownership before expanding runtime assumptions. #54/#37 and #55 remain bounded physical/diagnostic probes; #56 research does not change the adopted kernel model.

## Verified evidence

`cargo xtask check` passed: 259 workspace Rust tests plus 13 acceptance-feature tests, formatting, Clippy and normal/acceptance native builds. All 189 runner tests passed.

The separate task-write suite passed **two VM boots**: add/done/no-op, maximum 672-byte document, Full, human version conflict, prepared-but-unsubmitted intent, lost journal acknowledgement, actual effect-reply discard, service restart, reboot, an actual older numeric-key collision, later human edits, expired history and policy denial. Independent disk assertions verify exact bytes/receipts and no replay during refused recovery. Resource counters return to 52,081 free frames, 3 processes, 4 channels and zero pending I/O. Both guests stopped and reclaimed resources.

The full terminal regression also passed two boots / 2,327 phase-one commands, including 22 ordinary task cases, three lifecycle cases and 17 previews plus reboot checks. All four native contract validators passed. The command count varies with polling; terminal JSON is 54,205 bytes within its 64 KiB bound.

A separate normal-image boot passed ordinary add/done/list with restored resources and rejected both acceptance commands. Normal build `2db61cc45e79820d`, kernel SHA-256 `736ceee6c6c2d5c8cc789c011778b3492b053e034e2a2f5371b6bcdd5946d794`; local evidence is `artifacts/tasks-write-normal/evidence.json` and its serial log.

Acceptance build `96a8478f31182d9c`; kernel SHA-256 `9640e9a8985df89cdf8458c70e1df2c4531a30f623a50d72db46da58ae080566`; tasks ELF SHA-256 `61033f5274e3100c7d8049727f3ba4f619bfa5608819a8f273d06c00e2d5efe0`. Evidence: `artifacts/tasks-write-check.log`, `tasks-write-runner-tests.log`, `tasks-write-native.log`, `tasks-write-terminal.log`, `tasks-write-*-conformance.json`, `tasks-write-test/tasks-write.json` and `terminal-test/terminal.json`. Full remote CI and a fresh isolated-container run are not claimed.

Luna max investigated durable identity and implemented candidate transport; root integrated persistence, tests and compiler fixes and owned all shared builds/VMs. Astra low reviewed the actual diff and final failure cuts/evidence with no material findings. Runtime maximum-document execution passed; static compiled stack headroom is not independently certified by that review.

## Workspace constraints

Never experiment on `artifacts/terminal/data.raw`. Preserve the existing owner modification of `LICENSE`; do not stage it incidentally. Maintained docs remain English. Continue using the project orchestration skill: Luna max bounded workers, Astra low review, at most two concurrent children and one owner for shared build/VM tests.
