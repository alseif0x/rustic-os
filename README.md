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

**Early, experimental kernel development.** RusticOS boots its own modular Rust kernel in QEMU through Limine and UEFI. It already runs isolated test programs in user mode and exchanges messages between processes. There is no interactive shell, desktop, browser, or integrated agent yet.

| Available today | What is verified |
| --- | --- |
| [UEFI boot and diagnostics](docs/BOOT.md) | Image construction, serial output, positive boot and deliberate failures |
| [Interrupts and time](docs/INTERRUPTS.md) | CPU exception handling, emergency stack, timer interrupts and bounded waits |
| [Memory protection](docs/MEMORY.md) | Physical frames, owned page tables, write/execute permissions and separate address spaces |
| [Native processes](docs/PROCESSES.md) | Static ELF loading, ring 3 execution, timer preemption, fault containment and resource reclamation |
| [Native Rust SDK](docs/SDK.md) | Independent Rust ELF applications, versioned manifests and typed IPC clients |
| [IPC and handles](docs/IPC.md) | Versioned messages, kernel-provided sender identity, validated buffers, ownership, waits and closure |
| [Block storage](docs/BLOCK.md) | Bounded VirtIO reads/writes, flush, restart persistence, device errors and DMA recovery |
| [Isolated test executor](docs/EXECUTOR.md) | Exact Git revisions, offline jobs, resource limits, cancellation and structured evidence |

The current acceptance suite covers **37 Rust tests, 26 Python tests, 18 VM scenarios and 22 isolated executor scenarios**, including user-mode exchanges and disk persistence across separate VM boots. See [storage acceptance](https://github.com/alseif0x/rustic-os/issues/35) and [GitHub Actions](https://github.com/alseif0x/rustic-os/actions/workflows/check.yml). These are checks of the reference configuration, not production security guarantees.

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

The kernel does not depend on MCP or a model. The service, tool and agent layers above it are roadmap work. Read the [agent integration proposal](docs/architecture/agent-integration.md) for the design and open experiments.

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

# Build the image and run the guest's acceptance checks.
python3 tools/boot.py run --mode ok
```

A successful run produces serial evidence and exits; it does not open an interactive desktop. Build artifacts and logs are written under `artifacts/boot/ok/`.

For the complete VM suite:

```sh
python3 tools/boot.py test --timeout 30
```

See [boot commands and expected results](docs/BOOT.md) and the [isolated executor](docs/EXECUTOR.md) for reproducible failure tests and revision-based execution.

## Built to stay modular

| Location | Responsibility |
| --- | --- |
| [`kernel/`](kernel/) | Pure kernel contracts plus architecture-specific boot, memory and process execution |
| [`crates/abi/`](crates/abi/) | Shared `no_std` binary contracts, independent of kernel implementation |
| [`crates/sdk/`](crates/sdk/) | Native application startup, process calls and typed IPC clients |
| [`apps/sdk-probe/`](apps/sdk-probe/) | Independently compiled Rust application and manifest template |
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

**Next:** [define authority](https://github.com/alseif0x/rustic-os/issues/5) and [service contracts](https://github.com/alseif0x/rustic-os/issues/6), then provide [bounded block access](https://github.com/alseif0x/rustic-os/issues/44) to the user-mode [file service](https://github.com/alseif0x/rustic-os/issues/12). The native SDK and sector storage are available; the service bridge, file permissions, supervision and shell remain ahead.

The experimental v0.1 target includes a native console, optional agent, locally running browser engine and a verifiable change/recovery cycle. The model, compiler and test environment may be external, with that dependency declared. Broad hardware support and universal application compatibility are long-term research goals, not current promises.

Follow the [living plan](https://github.com/alseif0x/rustic-os/issues/1), [milestones](https://github.com/alseif0x/rustic-os/milestones) and [long-term experiments](https://github.com/alseif0x/rustic-os/issues/31). Historical issue discussions may be in Spanish; repository documentation and new contribution templates use English.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md), the [requirements](docs/requirements-v0.1.md) and an issue with resolved dependencies. Architecture improvements are welcome when they include alternatives, maintenance costs and a testable outcome. Human and AI-assisted changes follow the same review and evidence requirements.

## License

Original RusticOS code and documentation are licensed under the **[Apache License 2.0](LICENSE)**. Third-party components retain their own licenses and notices. See the [licensing policy](docs/LICENSING.md) and [dependency inventory](docs/dependencies.md).
