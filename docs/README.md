<!-- SPDX-License-Identifier: Apache-2.0 -->

# RusticOS documentation

Start with the [project overview](../README.md) for current capabilities and the roadmap. These documents distinguish implemented contracts from proposed product architecture.

## Build and run

| Guide | Purpose |
| --- | --- |
| [Development](DEVELOPMENT.md) | Reference environment, workspace layout and checks |
| [Boot and VM tests](BOOT.md) | Build an image, run QEMU and interpret evidence |
| [Isolated executor](EXECUTOR.md) | Test exact revisions with resource and network restrictions |

## Implemented kernel contracts

| Contract | Scope |
| --- | --- |
| [Interrupts and time](INTERRUPTS.md) | Exceptions, timer IRQs, deadlines and emergency handling |
| [Memory](MEMORY.md) | Frames, page tables, ownership and protection |
| [Processes](PROCESSES.md) | ELF loading, user mode, scheduling and fault containment |
| [Process ABI](PROCESS-ABI.md) | Calling convention, version, exit, identity and diagnostics |
| [IPC](IPC.md) | Message format, handles, buffers, waits and resource lifetime |

## Requirements and design

- [Experimental v0.1 requirements](requirements-v0.1.md): scope, reference profiles and acceptance scenarios.
- [Kernel and boot decision](architecture/ADR-0001-kernel-and-boot.md): chosen architecture, alternatives and review conditions.
- [Agent integration proposal](architecture/agent-integration.md): service APIs, tools, MCP and proposed experiments.
- [ADR template](architecture/ADR-template.md): record a new architecture decision.
- [Living implementation plan](https://github.com/alseif0x/rustic-os/issues/1): dependencies, milestones and completion evidence. Historical issue discussions may be in Spanish.

## Contribute and distribute

- [Contribution guide](../CONTRIBUTING.md)
- [Engineering rules](../AGENTS.md)
- [Licensing and provenance](LICENSING.md)
- [Dependency inventory](dependencies.md)

The repository documentation is maintained in English. Test logs and issue records provide evidence for specific revisions; they do not establish capabilities beyond the documented configuration and limits.
