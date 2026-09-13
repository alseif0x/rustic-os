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

The three native lifecycle gaps were closed in 8886f00: pending-grant cancellation, concurrent utility reservation and abandoned-result expiry before the next owner request. Their acceptance instrumentation is absent from ordinary builds. Local source-regression probes rejected lost reservation and request-only expiry.

Current bounded consumer: `tasks preview add PATH TITLE` and `tasks preview done PATH ID` run app-owned edit planning inside the native tasks ELF. They return the complete candidate, affected ID, changed flag and pinned source version, explicitly without writing. A preview is temporary and does not reserve a durable mutation identity. The shared file protocol and READ-only grant remain unchanged.

Next implementation gate for actual add/done: define durable initial retry-key allocation, a scoped trusted mutation subject, and retained intent/result ownership across a lost/poisoned child connection. Existing `admit_file` can persist exact candidate bytes and allocate an admission ID before execution, but requires explicit format/profile activation and a retained initial key. Direct `replace_file` only obtains its historical service instance after commitment; PID/ticks/static keys cannot satisfy pre-submit durable identity. Prefer using the existing admission/recovery mechanisms after resolving these application-owned choices. Do not silently reread/rebase a retry or call process-death uncertainty success. Preserve process/channel/client budgets; do not add an optional file-service profile to hide the gap. See [Tasks](TASKS.md).

Preserve immutable retry intent, human conflicts, exact effect verification and explicit uncertainty. Keep remaining #47/#43 correctness work finite; do not add optional file protocol profiles without a real consumer or demonstrated defect.

## Other work and constraints

- #48–#52 own RAM, allocation, bulk data, workspace capacity and independent application delivery. Proceed on real dependencies, without an MCP gate.
- Review #53 concurrency ownership before expanding mapping/runtime assumptions.
- One bounded probe alongside main implementation: #54 physical-profile preparation / #37 visible diagnostics; #55 PCI matching is separately bounded. #56 driver-boundary research does not change the adopted kernel model.
- No physical reference machine has yet been selected in these planning changes.
- Never use artifacts/terminal/data.raw for experiments. LICENSE has an existing owner modification; do not stage or alter it incidentally.

## Evidence and handoff

Current increment over 8886f00: app-owned immutable edit planner, bounded preview wire, version-pinned native snapshot, scoped read-only supervisor relay and shell presentation. No new file-service profile, quota, heap, unsafe boundary or dependency. #22 remains open; applying add/done, general discovery and full M1 remain unimplemented.

Validation: `cargo xtask check` passed (253 workspace Rust tests plus 9 tests with the acceptance feature, formatting, Clippy and native builds); 189 runner tests passed. `python3 tools/terminal_test.py` passed two boots / 2,527 phase-one commands, including 17 preview cases plus a post-reboot preview, the 22 ordinary task cases and three lifecycle cases. All four terminal contract validators passed. Tested acceptance build f93ab6d5ad161df1, kernel SHA-256 71dadde9d6f838b75389e5465b8d70a3f1b227eb3edba3dace1619b94af21849; tasks ELF SHA-256 d98475ad668250257be7699379df1e0b11be515e3a3221489df438df84010859. Local evidence: artifacts/tasks-preview-check.log, artifacts/tasks-preview-runner-tests.log, artifacts/tasks-preview-terminal.log, artifacts/tasks-preview-*-conformance.json and artifacts/terminal-test/terminal.json (54,203 bytes). Command counts vary with polling. Full CI and a new isolated-container run are not claimed here.

Preview assertions compare exact candidate rows and the source version against the independent disk oracle, require unchanged committed bytes and restored resources, and cover repeated add previews, done/no-op, missing/invalid IDs, malformed data/titles, empty/full documents, ID exhaustion, occupied child slots, policy denial and reboot. Read-only listing still returns the original document afterward.

Luna max explored the mutation/recovery boundary and implemented the pure planner. Root integrated native snapshot/preview transport, shell, host acceptance and compiler fixes, and owned all builds/VMs. Astra low reviewed the actual changes with no material static findings. All test processes completed and both guests reported stopped/reclaimed. Replace this checkpoint at the next handoff rather than appending history.
