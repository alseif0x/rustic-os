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

Read-only `tasks list PATH` was published in e8c065d, following the prior f522583 help increment. See [Tasks](TASKS.md) for format, bounds and current acceptance. The task app owns parsing/version-pinned reads; its independent contract crate owns wire/data validation; supervisor owns scoped provisioning and bounded results; shell owns syntax/presentation. The trusted catalog still embeds executables; this does not complete #52.

The three native lifecycle gaps now have an explicit acceptance client in the existing authenticated owner shell: pending-grant cancellation, concurrent utility reservation and abandoned-result expiry before the next owner request. Instrumentation is absent from ordinary builds. Local source-regression probes also reject lost reservation and request-only expiry.

Next bounded implementation: `tasks add PATH TITLE` and `tasks done PATH ID`, beginning with one immutable mutation intent and exact effect verification through the existing replacement SDK. Define the retained intent/result ownership and conflict/retry behavior before editing; do not silently reread/rebase a retry or call process-death uncertainty success. Preserve shell console ownership and the existing process/channel/client budgets. No optional file-service profile expansion is required by this checkpoint.

Preserve immutable retry intent, human conflicts, exact effect verification and explicit uncertainty. Keep remaining #47/#43 correctness work finite; do not add optional file protocol profiles without a real consumer or demonstrated defect.

## Other work and constraints

- #48–#52 own RAM, allocation, bulk data, workspace capacity and independent application delivery. Proceed on real dependencies, without an MCP gate.
- Review #53 concurrency ownership before expanding mapping/runtime assumptions.
- One bounded probe alongside main implementation: #54 physical-profile preparation / #37 visible diagnostics; #55 PCI matching is separately bounded. #56 driver-boundary research does not change the adopted kernel model.
- No physical reference machine has yet been selected in these planning changes.
- Never use artifacts/terminal/data.raw for experiments. LICENSE has an existing owner modification; do not stage or alter it incidentally.

## Evidence and handoff

Current increment over e8c065d: native lifecycle acceptance for the read-only tasks consumer. A feature-gated shell client drives real authenticated RPC; supervisor hooks hold a sent task grant and record actual drain/expiry. No new kernel, SDK or file-service protocol, quota, heap, unsafe boundary or dependency. #22 remains open; add/done, general discovery and full M1 remain unimplemented.

Validation: `cargo xtask check` passed (244 workspace Rust tests plus 7 tests with the acceptance feature, formatting, Clippy and native builds); 189 runner tests passed. `python3 tools/terminal_test.py` passed two boots / 2,425 phase-one commands, including 22 ordinary task cases, three lifecycle cases and a post-reboot query. All four terminal contract validators passed. Tested acceptance build 346e7b2b35529aab, kernel SHA-256 aa39d1ec348e27f444f900f926a39342afaef432d223ba26d665d4f31e477099; tasks ELF remains 74b4311a1dea6486101807ff7319502432710f19f97ce39360961fc5505b6128. Local evidence: artifacts/tasks-lifecycle-check-probe.log, artifacts/tasks-lifecycle-runner-tests.log, artifacts/tasks-lifecycle-terminal.log, artifacts/tasks-lifecycle-*-conformance.json and artifacts/terminal-test/terminal.json (49,480 bytes). Command counts vary with polling. Full CI and a new isolated-container run are not claimed here.

The expiry witness records cleanup at tick 3401 before the next owner query at 3501. All lifecycle cases restore 52,110 free frames, 3 processes, 4 channels and no pending I/O; committed disk hashes match. Two deliberately regressed native builds fail in the expected concurrent/expiry case; their source edits were restored before the final suite (artifacts/tasks-regression-*/). A normal build rejects the fixture command and passes ordinary listing with unchanged disk/resources: build b8b8db550b9bd884, kernel SHA-256 7863bec139ba1d2c039a34a0d591baf997fdf5663ebf6a9b78a44960f3b7daed (artifacts/tasks-normal-smoke/).

Luna max explored and implemented the bounded Rust fixture. Root integrated build profiles, host validation and compiler fixes, and owned all builds/VMs. Astra low reviewed the actual changes with no remaining material static findings; its suggested exact cancellation-history assertion is included. All test processes completed; successful guests reported stopped/reclaimed, and regression guests were terminated by their owning context. Replace this checkpoint at the next handoff rather than appending history.
