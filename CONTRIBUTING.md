<!-- SPDX-License-Identifier: Apache-2.0 -->

# Contributing to RusticOS

The [living plan](https://github.com/alseif0x/rustic-os/issues/1) defines requirements and execution rules. Issues specify outcomes, dependencies, deliverables, tests and limits. Milestones group acceptance criteria while allowing early research.

## Before implementing

Choose a task with resolved dependencies, agree on ownership and available review, and define a verifiable change. If it requires several large independent changes, split it while preserving links.

The plan can improve: explain the problem, proposal, alternatives, maintenance cost, experiment and success criteria. Update affected dependencies and requirements. Do not retain a decision merely because it appears in a document.

Run `cargo xtask check` as described in [the development guide](docs/DEVELOPMENT.md). It checks formatting, lints, host tests and the `no_std` build. Guest behavior needs the relevant [boot tests](docs/BOOT.md); host checks do not replace them. Apply the module/submodule and separation rules in [AGENTS.md](AGENTS.md).

## Change criteria

- Connect each first-party product capability to an API/tool, authority, state and an independent way to verify its effect.
- Add positive and negative tests at each boundary from its implementation: memory, IPC, devices, services, transport and tools.
- Link code or decisions, review/configuration, commands, results and limits. Distinguish host fixtures from real execution in RusticOS.
- Document invariants and review for every `unsafe` boundary.
- Keep service logic common to the console, GUI and agent; neither a model nor MCP grants privileges.
- Leave checkboxes pending until evidence exists. The same standard applies to human and AI-generated contributions.
- Write documentation, new issues and pull requests in English so contributors can follow the project. Existing historical discussions retain their context.

## Licensing and provenance

Original material is published under [Apache-2.0](LICENSE). Use `SPDX-License-Identifier: Apache-2.0` with the file's comment syntax: for example, a line comment in Rust or scripts and an HTML comment in Markdown. Do not break formats that do not allow comments.

Preserve copyright notices that reflect actual authorship; do not invent holders, remove attribution or replace third-party headers. Before adding a dependency, complete its record and review in [docs/LICENSING.md](docs/LICENSING.md).

There is no separate contributor agreement. Contribute only material you have the right to provide and disclose any content under different terms.
