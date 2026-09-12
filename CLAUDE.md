<!-- SPDX-License-Identifier: Apache-2.0 -->

# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Repository rules

@AGENTS.md

[AGENTS.md](AGENTS.md) is the authoritative set of engineering rules (modularity,
dependency direction, visibility, `unsafe` boundaries, validation and scope). It
applies to every change here; this file only adds the practical context Claude
needs to act on it.

Also required reading before non-trivial work:

- [CONTRIBUTING.md](CONTRIBUTING.md) — change criteria, evidence and licensing.
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) — environment and every runnable check.
- [docs/requirements-v0.1.md](docs/requirements-v0.1.md) and
  [docs/architecture/ADR-0001-kernel-and-boot.md](docs/architecture/ADR-0001-kernel-and-boot.md) — plan and scope.

## What this project is

RusticOS is an independent `no_std` Rust operating system for x86_64 UEFI, booted
with Limine under QEMU. The kernel is monolithic but modular; the supervisor,
file server, shell and utilities are separate user-mode ELF applications that
talk over versioned IPC.

## Layout

| Path | Contents |
| --- | --- |
| `kernel/` | `no_std` kernel: `arch`, `boot`, `memory`, `process`, `ipc`, `handles`, `block`, `drivers`, `time`. `lib.rs` is pure logic, `main.rs` the executable. |
| `crates/abi` | Shared ABI and message types. No implementations. |
| `crates/sdk` | Native user-mode SDK (typed IPC clients). |
| `crates/fs`, `crates/file-service` | Volume format and file-service logic. |
| `apps/` | Guest ELFs: `supervisor`, `file-server`, `shell`, `utility`, `sdk-probe`, `block-probe`. |
| `contracts/services` | Logical service-v1 schemas and descriptors. |
| `tools/` | Host Python harnesses (`boot.py`, `terminal.py`, `application.py`, `sandbox.py`, `measure.py`, `contracts/`) and `xtask`. |
| `docs/` | One guide per subsystem; each states what is actually verified and its limits. |
| `artifacts/` | Recorded evidence from runs. Do not hand-edit. |

Dependency direction is one-way: the kernel never depends on the SDK, apps,
tools or host code.

## Commands

The verified baseline is Ubuntu 24.04 (WSL2 on Windows) with Rust 1.98.1 pinned
by `rust-toolchain.toml`. Run these from the repository root inside Ubuntu:

```sh
cargo xtask check          # fmt, clippy -D warnings, host tests, no_std build + kernel-target clippy
cargo fmt --all            # apply formatting
python3 tools/application.py                      # build guest ELFs and manifests
python3 tools/boot.py run --mode recovery-test --timeout 60
python3 tools/boot.py run --mode block-user --timeout 45
python3 tools/terminal_test.py                    # native terminal scenarios
python3 tools/sandbox.py prepare                  # rebuild isolated executor infrastructure
```

Host-only contract checks need the pinned validator environment in
`.cache/contracts-venv`; see [docs/SERVICE-CONTRACTS.md](docs/SERVICE-CONTRACTS.md).
`cargo xtask check` needs Python 3.11+ but not QEMU.

## Working rules for Claude

- Never report a check as passing without running it. Host tests and host
  fixtures do not demonstrate guest behavior — say which one you ran.
- Do not tick checkboxes, mark an issue done or claim a capability exists
  because documentation mentions it. Evidence first.
- Keep every new file's `SPDX-License-Identifier: Apache-2.0` header in the
  comment syntax of its format.
- Documentation, issues, commit messages and pull requests are written in
  English, even when the conversation is in another language.
- Add behavior and failure tests in the layer that owns the contract; a test
  that restates the implementation is not useful here.
- Every new `unsafe` block needs a documented safety comment (`clippy::undocumented_unsafe_blocks` is denied).
- `artifacts/` and `docs/evidence/` record real runs; regenerate them with the
  harness instead of editing them.
