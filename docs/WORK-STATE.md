<!-- SPDX-License-Identifier: Apache-2.0 -->

# Current work state

Updated 2026-09-13. Keep this file compact; replace current state rather than appending a conversation log.

## Baseline and direction

- Orchestration baseline: a9f187f3dea9907c55972dbaab962902a564d27c (Astra medium coordination, Luna max execution, Astra low review). Prior runtime baseline: 0859e9b.
- Own modular monolithic Rust OS; isolated native product services, human and agent clients sharing authority and semantics. MCP is an optional adapter.
- Multiple preempted processes on one executing CPU. SMP, independent application installation and physical-machine support remain unimplemented.
- Current execution order: [issue #1](https://github.com/alseif0x/rustic-os/issues/1) and [systems roadmap](architecture/systems-roadmap.md). Historical issue updates are evidence for their revisions, not current next-step instructions.

## Next implementation

[#22](https://github.com/alseif0x/rustic-os/issues/22): a parameterized native tasks application with ordinary add/done/list, stable task IDs, scoped authority and application-owned semantics using existing file services. Define one small acceptance slice before editing; this file is not a substitute for the current issue contract.

Read-only `tasks list PATH` is now implemented in a separate native ELF, following the prior f522583 help increment. See [Tasks](TASKS.md) for format, bounds and current acceptance. The task app owns parsing/version-pinned reads; its independent contract crate owns wire/data validation; supervisor owns scoped provisioning and bounded results; shell owns syntax/presentation. The trusted catalog still embeds executables; this does not complete #52.

Next bounded acceptance: exercise cancellation during grant provisioning, an ordinary utility launched while that slot is reserved, and abandoned-result expiry through an actual deterministic owner client in the guest. The synchronous shell suite does not interleave those requests. Do not substitute cancellation during shell path resolution for task-job cancellation. Then implement add/done with immutable retry/reconciliation semantics. Preserve shell console ownership and the existing process/channel/client budgets.

Preserve immutable retry intent, human conflicts, exact effect verification and explicit uncertainty. Keep remaining #47/#43 correctness work finite; do not add optional file protocol profiles without a real consumer or demonstrated defect.

## Other work and constraints

- #48–#52 own RAM, allocation, bulk data, workspace capacity and independent application delivery. Proceed on real dependencies, without an MCP gate.
- Review #53 concurrency ownership before expanding mapping/runtime assumptions.
- One bounded probe alongside main implementation: #54 physical-profile preparation / #37 visible diagnostics; #55 PCI matching is separately bounded. #56 driver-boundary research does not change the adopted kernel model.
- No physical reference machine has yet been selected in these planning changes.
- Never use artifacts/terminal/data.raw for experiments. LICENSE has an existing owner modification; do not stage or alter it incidentally.

## Evidence and handoff

Current increment: separate tasks ELF, `tasks list PATH`, bounded complete document validation and scoped read-only execution. Native assertions cover normal/empty/maximal lists, malformed tails/IDs, title/count bounds, denied policy, full child slots, reuse and reboot. Every query compares the full committed disk view and process/channel/frame counters. #22 remains open; add/done, general discovery and full M1 remain unimplemented.

Validation: `cargo xtask check` passed (244 Rust tests, formatting, Clippy and native builds); 180 runner tests passed. `python3 tools/terminal_test.py` passed two boots / 2,238 phase-one commands, including 22 task cases plus a post-reboot query and the independent disk oracle. All four terminal contract validators passed. Tested build 13cd98702a24c9fe, kernel SHA-256 50d533d8ac51efc50271a7cb5d1398ba1eb1b25dfa70f3585a792a6f3f662495. Tasks ELF SHA-256 74b4311a1dea6486101807ff7319502432710f19f97ce39360961fc5505b6128. Local evidence: artifacts/tasks-check.log, artifacts/tasks-runner-tests.log, artifacts/tasks-terminal.log, artifacts/tasks-*-conformance.json and artifacts/terminal-test/terminal.json (48,247 bytes). Command counts vary with polling. Full CI for this increment is not claimed here.

Luna max implemented app/contracts/supervisor; root integrated shell/builds/tests. Astra low found slot-reservation, idle-expiry and ownership/compilation issues; the fixes were reviewed with no remaining material static findings. A documented Clippy expectation retains one bounded inline job without introducing a heap/global buffer. The three native lifecycle cases named above remain evidence gaps. Root owned shared builds/VMs; all test processes completed and both guests reported stopped/reclaimed. Replace this checkpoint at the next handoff rather than appending history.
