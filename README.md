<!-- SPDX-License-Identifier: Apache-2.0 -->

# RusticOS

**An independent Rust operating system, designed for people and AI agents.**

[![CI](https://github.com/alseif0x/rustic-os/actions/workflows/check.yml/badge.svg?branch=main)](https://github.com/alseif0x/rustic-os/actions/workflows/check.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Stage: experimental](https://img.shields.io/badge/stage-experimental-orange.svg)](#current-status)
[![Target: x86_64 UEFI](https://img.shields.io/badge/target-x86__64%20UEFI-555.svg)](docs/requirements-v0.1.md)

[Get started](#get-started) · [Documentation](docs/README.md) · [Roadmap](#roadmap) · [Contribute](CONTRIBUTING.md)

RusticOS explores what an operating system can become when its capabilities are accessible through structured APIs from the start. People and agents should be able to discover capabilities, inspect state, perform authorized operations, and verify the result through the same underlying services.

The goal is agent control without mandatory screen interpretation. A graphical interface and visual automation remain complementary options. AI is optional: manual use and recovery must work without a model.

## Current status

**A working native terminal on an experimental Rust OS.** RusticOS boots its own modular kernel in QEMU through Limine and UEFI. A separate supervisor, file server and shell run in user mode: enter commands, save files across reboots, inspect processes and revoke scoped utility access. A desktop, browser and integrated agent remain roadmap work.

| Available today | What is verified |
| --- | --- |
| [UEFI boot and diagnostics](docs/BOOT.md) | Image construction, serial output, positive boot and deliberate failures |
| [Interrupts and time](docs/INTERRUPTS.md) | CPU exception handling, emergency stack, timer interrupts and bounded waits |
| [Memory protection](docs/MEMORY.md) | Physical frames, owned page tables, write/execute permissions and separate address spaces |
| [Native processes](docs/PROCESSES.md) | Static ELF loading, ring 3 execution, timer preemption, fault containment and resource reclamation |
| [Native Rust SDK](docs/SDK.md) | Independent Rust ELF applications, versioned manifests and typed IPC clients |
| [IPC and handles](docs/IPC.md) | Versioned messages, kernel-provided sender identity, validated buffers, ownership, waits and closure |
| [Block storage](docs/BLOCK.md) | Bounded VirtIO reads/writes, flush, restart persistence, device errors and DMA recovery |
| [User-mode disk access](docs/BLOCK-ACCESS.md) | Typed SDK, scoped handles, asynchronous sector I/O, cancellation and process-death recovery |
| [Native terminal](docs/TERMINAL.md) | Real keyboard input, file commands, isolated utilities, permissions, service restart and reboot persistence |
| [File service](docs/FILES.md) | Bounded copy-on-write volume, version checks, scopes, recovery model and independent disk verification |
| [Client/helper authority](docs/AUTHORITY.md) | Checked subsets, shared revocation, moved-handle denial, inherited expiry and owner control during queue pressure or a stopped file service |
| [Workspace operations](docs/FILE-OPERATIONS.md) | Scoped retry keys, original SHA-256 receipts, lost-response lookup, explicit format migration and native I/O fault recovery |
| [Explicit durable operations](docs/FILE-ADMISSION-API.md) | Prepare without executing, recover acceptance after a lost reply, inspect, execute or cancel through SDK and terminal |
| [Isolated test executor](docs/EXECUTOR.md) | Exact Git revisions, offline jobs, resource limits, cancellation and structured evidence |

The acceptance suite combines Rust contract tests, Python runner checks, native VM scenarios and isolated executor scenarios, including user-mode exchanges and disk persistence across separate VM boots. See [terminal acceptance and limits](docs/TERMINAL.md) and [GitHub Actions](https://github.com/alseif0x/rustic-os/actions/workflows/check.yml) for procedures and revision-specific results. These check the reference configuration; they do not establish production security guarantees. The separate [service contract suite](docs/SERVICE-CONTRACTS.md) checks logical schemas and descriptors on the host.

[Native files.read](docs/FILES-READ.md) and [workspace replacements/completed operations](docs/FILE-OPERATIONS.md) connect three logical methods to the real file service, shell and shared SDK. Reads verify the version and returned range; replacements retain an original SHA-256 receipt that can be queried after a lost reply, later edit or restart. Stable references grant no authority. Shared host/native fixtures cover these bounded profiles; the complete eight-operation catalog remains planned.

The [measurement harness](docs/MEASUREMENTS.md) adds repeated native resource/control workloads, a held-out unchanged batch and a real delayed-service regression. Its budgets are derived from a compatible baseline; failed samples and different hosts cannot be pooled.

The current VM uses **x86_64, one CPU and 256 MiB RAM**. Process and IPC limits are deliberately small; see their contracts before building on them.

## Why an API-accessible OS?

An agent should be able to ask a service what it can do, submit a typed request, observe progress and verify the outcome. That requires explicit contracts and permissions throughout the system.

The planned integration follows these layers:

```text
People and replaceable AI agents
        │
Console / GUI / native client / MCP adapter
        │
Task-oriented tools and versioned service APIs
        │
OS services and native application SDK
        │
Rust kernel: memory · processes · IPC · isolation
```

- **Shared services:** the console, GUI and agents use the same service logic.
- **Explicit authority:** discovering a capability does not grant permission to use it.
- **MCP interoperability:** MCP is a planned adapter; native clients can use the local contracts directly.
- **Verifiable effects:** operations need inspectable state, meaningful errors and independent outcome checks.
- **Measured adaptability:** hardware, application compatibility and interaction modes expand through tested capabilities and deterministic fallback policies.

The kernel does not depend on MCP or a model. Native file/supervision services support the manual terminal, and the shell and deterministic client share typed read, replacement and operation-lookup SDK paths. The first eight [shared service contracts](docs/SERVICE-CONTRACTS.md) have checked schemas and generated descriptors; three methods have bounded native bindings. General asynchronous operations, discovery, tools and agents remain roadmap work. [Agent integration](docs/architecture/agent-integration.md) records the adapter design and open experiments.

The [systems research agenda](docs/architecture/systems-roadmap.md) explores a further goal: tasks whose state, authority, effects and recovery remain understandable across people and replaceable agents. It includes prior work, stage gates and a small executed host model; these proposals are distinct from the implemented features above.

## Get started

Use **Ubuntu 24.04 amd64**, directly or through WSL2. Ubuntu hosts the compiler and QEMU; the guest runs RusticOS's own kernel. Install Rust and the base tools using the [development guide](docs/DEVELOPMENT.md), then run:

```sh
git clone https://github.com/alseif0x/rustic-os.git
cd rustic-os
source ~/.cargo/env

# Install the pinned reference VM tools; requires sudo.
python3 tools/environment.py install

# Check the workspace and host-side runner contracts.
cargo xtask check
python3 -m unittest discover -s tools/tests -v

# First interactive launch: create a new dedicated sparse data image.
python3 tools/terminal.py --initialize

# Later launches: mount the same persistent disk.
python3 tools/terminal.py
```

Type help in the native terminal. Try write hello "Hello from RusticOS", cat hello, ps, services and mem; exit stops the VM. The dedicated disk stays in artifacts/terminal/data.raw. See [terminal syntax, permissions and recovery](docs/TERMINAL.md). Initialization refuses an existing disk.

For the complete VM suite:

```sh
python3 tools/boot.py test --timeout 45
```

See [boot commands and expected results](docs/BOOT.md) and the [isolated executor](docs/EXECUTOR.md) for reproducible failure tests and revision-based execution.

## Built to stay modular

| Location | Responsibility |
| --- | --- |
| [`kernel/`](kernel/) | Pure kernel contracts plus architecture-specific boot, memory and process execution |
| [`crates/abi/`](crates/abi/) | Shared `no_std` binary contracts, independent of kernel implementation |
| [`crates/sdk/`](crates/sdk/) | Native entry, runtime/console, IPC/block and file-service clients |
| [`crates/fs/`](crates/fs/) · [`crates/file-service/`](crates/file-service/) | Pure volume format and bounded file authority/staging, independent of kernel and SDK |
| [`apps/`](apps/) | Independent supervisor, file server, shell and utility applications, plus acceptance probes |
| [`contracts/`](contracts/services/v1/catalog.json) | Versioned logical service schemas, descriptor metadata and positive/negative examples |
| [`tools/`](tools/) | Host-side checks, image construction, QEMU execution and sandbox orchestration |
| [`docs/`](docs/README.md) | Requirements, architecture decisions, subsystem contracts and evidence guides |

Modules follow ownership and trust boundaries. Entry points compose components; CPU instructions and `unsafe` code stay in narrow modules with documented invariants. See [the repository engineering rules](AGENTS.md).

## Roadmap

| Milestone | Outcome | Status |
| --- | --- | --- |
| H0 — Reproducible foundation | Licensed workspace, boot and failure detection | Complete |
| H1 — Manual OS foundation | Isolated processes, IPC, native SDK, persistent files, authority and shell | In progress |
| H2 — Programmatic control | Service tools, operation contracts and capability-based adaptation | Planned |
| H3 — Networking and agents | Networking, MCP interoperability and an optional integrated agent | Planned |
| H4 — Desktop and browser | Graphical interaction and a browser engine running inside RusticOS | Planned |
| H5 — Experimental v0.1 | Verified candidates, activation, recovery and integrated acceptance | Planned |

[Workspace replacements and completed operations](docs/FILE-OPERATIONS.md) now provide scoped retry keys, immutable historical receipts and lookup after a lost response or restart. [Interruptible owner control](docs/FOREGROUND-CONTROL.md) keeps supervision available during submitted I/O, while [stable reads](docs/FILES-READ.md) verify the observed file. [Explicit admissions](docs/FILE-ADMISSION-API.md) add durable preparation, status, execution and separate cancellation authority through native IPC. Pending work never resumes automatically after restart. **Next:** connect bounded background execution and in-flight public cancellation, then the remaining service-v1 methods in [#12](https://github.com/alseif0x/rustic-os/issues/12)/[#22](https://github.com/alseif0x/rustic-os/issues/22) and remaining takeover cases in [#13](https://github.com/alseif0x/rustic-os/issues/13). The [measurement harness](docs/MEASUREMENTS.md) evaluates resource cost. This remains a bounded prototype with shared two-record retention and no general delegation or production filesystem guarantees.

The experimental v0.1 target includes a native console, optional agent, locally running browser engine and a verifiable change/recovery cycle. The model, compiler and test environment may be external, with that dependency declared. Broad hardware support and universal application compatibility are long-term research goals, not current promises.

Follow the [living plan](https://github.com/alseif0x/rustic-os/issues/1), [milestones](https://github.com/alseif0x/rustic-os/milestones) and [long-term experiments](https://github.com/alseif0x/rustic-os/issues/31). Historical issue discussions may be in Spanish; repository documentation and new contribution templates use English.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md), the [requirements](docs/requirements-v0.1.md) and an issue with resolved dependencies. Architecture improvements are welcome when they include alternatives, maintenance costs and a testable outcome. Human and AI-assisted changes follow the same review and evidence requirements.

## License

Original RusticOS code and documentation are licensed under the **[Apache License 2.0](LICENSE)**. Third-party components retain their own licenses and notices. See the [licensing policy](docs/LICENSING.md) and [dependency inventory](docs/dependencies.md).
