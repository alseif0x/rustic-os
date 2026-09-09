<!-- SPDX-License-Identifier: Apache-2.0 -->

# Working on RusticOS

These rules apply throughout the repository. The owner requires modular Rust with submodules and separation of responsibilities.

## Design and implementation

- Organize by responsibility and trust boundary: boot, architecture, memory, processes, IPC, drivers and services. Create submodules for independent concepts; do not place unrelated subsystems in one file.
- `main.rs` and `lib.rs` are entry and composition points. `mod.rs` or the module root declares structure, public API and minimal coordination; it must not accumulate subsystem implementations.
- Keep details private by default. Use `pub(super)` or `pub(crate)` where sufficient; expose a small public API with defined types and errors.
- Keep dependencies directed and acyclic. The kernel must not depend on the user SDK, a model, MCP, GUI or host tools. Shared contracts must not import implementations.
- Separate mechanism from policy: the kernel enforces memory, handles and isolation; services make product policy decisions within that authority.
- Encapsulate CPU instructions, MMIO/port I/O and `unsafe` in narrow modules; document validity, ownership, lifetime and concurrency invariants.
- Avoid `utils`/`common` modules that mix responsibilities, managers that know everything, and mutable global state without an owner. Each subsystem's state has an explicit owner and access rules.
- Extract a crate when there is a useful reuse, platform, trust or compilation boundary. Do not create one crate per file or abstractions/traits without a real need.
- Separate host code for builds/tests from guest code. Do not accidentally introduce `std` into the `no_std` kernel.
- Split by cohesion and reasons for change, not an arbitrary line limit. Do not create empty trees for future functionality.
- A monolithic kernel can share privileged address space while remaining modular in code. Modularity does not establish driver isolation.

## Review and validation

For every Rust change, review module responsibilities, dependency direction, visibility, state ownership and new `unsafe` boundaries. Add behavior/failure tests in the layer that owns the contract; avoid tests that merely repeat the implementation.

Run configured formatting, lints and tests appropriate to the change. Use the commands established in [the development guide](docs/DEVELOPMENT.md); never invent successful results. Boot, contracts and authority require different validation; a host mock does not demonstrate execution in RusticOS.

## Plan and scope

Consult `docs/requirements-v0.1.md`, `docs/architecture/ADR-0001-kernel-and-boot.md` and the active issue. Preserve traceability when revising decisions. Do not claim capabilities are implemented or close tests merely because documentation exists. Follow `CONTRIBUTING.md` and `docs/LICENSING.md` for provenance and notices.

Write maintained documentation and contribution templates in English. Preserve protocol identifiers, commands, source references and historical evidence when translating.
