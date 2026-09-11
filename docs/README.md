<!-- SPDX-License-Identifier: Apache-2.0 -->

# RusticOS documentation

Start with the [project overview](../README.md) for current capabilities and the roadmap. These documents distinguish implemented contracts from proposed product architecture.

## Build and run

| Guide | Purpose |
| --- | --- |
| [Development](DEVELOPMENT.md) | Reference environment, workspace layout and checks |
| [Native terminal](TERMINAL.md) | Interactive commands, persistent disk, process control, permissions and recovery |
| [Boot and VM tests](BOOT.md) | Build an image, run QEMU and interpret evidence |
| [Isolated executor](EXECUTOR.md) | Test exact revisions with resource and network restrictions |

## Implemented kernel contracts

| Contract | Scope |
| --- | --- |
| [Interrupts and time](INTERRUPTS.md) | Exceptions, timer IRQs, deadlines and emergency handling |
| [Memory](MEMORY.md) | Frames, page tables, ownership and protection |
| [Processes](PROCESSES.md) | ELF loading, user mode, scheduling and fault containment |
| [Process ABI](PROCESS-ABI.md) | Calling convention, version, exit, identity and diagnostics |
| [Native SDK and manifests](SDK.md) | Application entry, public IPC/block clients, versioned admission and runnable Rust templates |
| [Block storage](BLOCK.md) | Bounded VirtIO I/O, DMA ownership, disposable disks and restart persistence |
| [User-mode block access](BLOCK-ACCESS.md) | Copied asynchronous requests, scoped handles, cancellation and safe DMA lifetime |
| [IPC](IPC.md) | Message format, handles, buffers, waits and resource lifetime |

## Native service foundations

| Contract | Scope |
| --- | --- |
| [Session runtime](NATIVE-RUNTIME.md) | Owned process/control calls, console, wait sets and bounded capacity |
| [File service](FILES.md) | Original volume format, native protocol, scopes, persistence and explicit limits |
| [Stable references and reads](FILES-READ.md) | Shared shell/client SDK, version-pinned ranges, SHA-256 and native contract evidence |
| [Native authority](AUTHORITY.md) | Explicit client/helper subsets, shared revocation, moved handles and owner control under pressure |
| [Workspace operations](FILE-OPERATIONS.md) | Completed replacements, scoped retries, original result lookup, explicit format upgrade and native fault evidence |
| [Recoverable replacements](FILE-RECOVERY.md) | Atomic file/version receipts, bounded retries, identity, legacy upgrade and native I/O fault tests |
| [Durable admission storage](FILE-ADMISSION.md) | Persistent pre-effect identity, cancellation records, explicit format-4 migration and restart evidence |
| [Public admission API](FILE-ADMISSION-API.md) | Explicit durable preparation, status, execution and independent cancellation through SDK/terminal |
| [Admission control](FILE-ADMISSION-CONTROL.md) | Pollable service admission, fresh execution authority and durable cleanup after owner revocation |

## Requirements and design

- [Experimental v0.1 requirements](requirements-v0.1.md): scope, reference profiles and acceptance scenarios.
- [Kernel and boot decision](architecture/ADR-0001-kernel-and-boot.md): chosen architecture, alternatives and review conditions.
- [Minimum shared authority decision](architecture/ADR-0002-authority-and-delegation.md): resource/action grants, session/helper scope, owner control and the removal of mandatory permission tiers; [20 reviewed cases](architecture/authority-cases.md) track runtime checks and remaining obligations.
- [Shared service contracts](SERVICE-CONTRACTS.md): eight specified operations, executable schema/examples and generated descriptors; [ADR-0003](architecture/ADR-0003-service-contracts.md) explains the decision and [16 scenarios](architecture/service-contract-cases.md) assign future stateful/guest conformance.
- [Agent integration proposal](architecture/agent-integration.md): service APIs, tools, MCP and proposed experiments.
- [Systems roadmap and research agenda](architecture/systems-roadmap.md): stage gates, architectural gaps and bounded experiments for human/agent workflows.
- [H1 file acceptance](FILE-FOUNDATION-ACCEPTANCE.md): original storage criteria, native/host evidence and the separate H2 lifecycle scope.
- [Early browser feasibility](architecture/browser-feasibility.md): candidate investigation, runtime gaps and bounded experiment gates.
- [Operation-boundary experiment](architecture/operation-model.md): finite host model of edit/revocation races and retry after a crash; not guest implementation evidence.
- [ADR template](architecture/ADR-template.md): record a new architecture decision.
- [Living implementation plan](https://github.com/alseif0x/rustic-os/issues/1): dependencies, milestones and completion evidence. Historical issue discussions may be in Spanish.

## Contribute and distribute

- [Contribution guide](../CONTRIBUTING.md)
- [Engineering rules](../AGENTS.md)
- [Licensing and provenance](LICENSING.md)
- [Dependency inventory](dependencies.md)

The repository documentation is maintained in English. Test logs and issue records provide evidence for specific revisions; they do not establish capabilities beyond the documented configuration and limits.
