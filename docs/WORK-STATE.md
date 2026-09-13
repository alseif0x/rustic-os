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

Start the application with a read-only `tasks list` slice in a separate native ELF; add mutations afterward with the required immutable retry/reconciliation semantics. Integration owners: `tools/application.py` and the kernel native catalog for artifacts/admission; `apps/supervisor/src/work/launch.rs` and child collection for scoped launch/results; shell commands for forwarding/presentation; the task app for its document format and stable IDs. The current launcher hard-codes UTILITY and has no generic child relay. Preserve shell console ownership and the existing process/channel/client budgets. Reusing the diagnostic utility would not establish a separate application.

Preserve immutable retry intent, human conflicts, exact effect verification and explicit uncertainty. Keep remaining #47/#43 correctness work finite; do not add optional file protocol profiles without a real consumer or demonstrated defect.

## Other work and constraints

- #48–#52 own RAM, allocation, bulk data, workspace capacity and independent application delivery. Proceed on real dependencies, without an MCP gate.
- Review #53 concurrency ownership before expanding mapping/runtime assumptions.
- One bounded probe alongside main implementation: #54 physical-profile preparation / #37 visible diagnostics; #55 PCI matching is separately bounded. #56 driver-boundary research does not change the adopted kernel model.
- No physical reference machine has yet been selected in these planning changes.
- Never use artifacts/terminal/data.raw for experiments. LICENSE has an existing owner modification; do not stage or alter it incidentally.

## Evidence and handoff

Current increment: ordinary `help`, preserved diagnostics in `help advanced`, and argument validation before output. Native assertions cover catalog separation, success/error status and subsequent shell use. #22 remains open; the tasks application is not yet implemented.

Validation: `cargo xtask check` passed (239 Rust tests, formatting, Clippy and native builds); `python3 tools/terminal_test.py` passed two boots and 2,143 phase-one commands with the independent disk oracle. Tested build a3aa887ef5bd6d1c, kernel SHA-256 6785a8b5e8f4b7da4c5f41436093dccd0ebee34154cceda8ccc69124da53d399. Local evidence: artifacts/orchestration-help-check.log, artifacts/orchestration-help-terminal.log and artifacts/terminal-test/terminal.json. Command counts vary with polling. Full CI for this increment is not claimed here.

Luna max performed bounded exploration and implementation; Astra low reviewed the final diff without material findings, including exact preservation of the old help text. Root owned shared builds/VMs; both test commands completed and the guest reported stopped/reclaimed. At the next handoff replace these results with the new revision/evidence and next action, rather than appending history.
