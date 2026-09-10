<!-- SPDX-License-Identifier: Apache-2.0 -->

# Development and checks

## Environment

Verified baseline: Ubuntu 24.04 amd64 (including WSL2), Rust 1.98.1 and rustup 1.29.1. On Windows, run the commands below inside Ubuntu. The checkout may live under /mnt/c; a checkout on the Linux filesystem can improve I/O performance.

Install the basic tools with apt (administrator privileges required):

```sh
sudo apt-get update
sudo apt-get install -y build-essential curl python3 python3-venv git
```

If rustup is missing, follow the [official installation guide](https://rust-lang.github.io/rustup/installation/index.html). This baseline used the official installer with --profile minimal --no-modify-path. Activate the environment with:

```sh
. "$HOME/.cargo/env"
rustup toolchain install 1.98.1 --profile minimal --component rustfmt --component clippy --target x86_64-unknown-none
```

rust-toolchain.toml selects this exact version; nightly is not required. Cargo.lock pins the Limine bindings and bitflags, used only by the boot executable. Checks use --locked. rust-lld produces a static ELF through kernel/linker.ld; the pure library remains separate from the executable.

## Runnable commands

From the repository root:

```sh
cargo xtask check
```

This runs formatting, Clippy with warnings treated as errors, host tests, the no_std build and Clippy for the kernel target, sequentially. To apply formatting:

```sh
cargo fmt --all
```

The kernel library validates arithmetic on memory ranges supplied during boot. Its tests run on the host; they do not demonstrate guest memory initialization, isolation or boot.

## Organization

- kernel/src/lib.rs: entry and composition for the no_std library.
- kernel/src/boot/mod.rs: boot-data module facade.
- kernel/src/boot/region.rs: range validation; no Limine dependency or memory allocation.
- kernel/tests/boot_regions.rs: public contract and edge-case tests.
- tools/xtask/src/main.rs: host utility entry point.
- tools/xtask/src/checks.rs: check sequence.
- tools/xtask/src/command.rs: process execution and error propagation.

There are no empty directories for future subsystems. Modularity rules are in [AGENTS.md](../AGENTS.md). The library's forbid(unsafe_code) applies to pure logic; privileged boundaries belong in appropriate architecture modules with explicit review.

## Boot tools

These are not required for cargo xtask check. The reference tools established for #8 are:

```sh
python3 tools/environment.py install
python3 tools/environment.py verify
python3 tools/environment.py fetch-bootloader
```

install installs the exact versions in tools/environment.toml; verify rejects different packages or firmware hashes and checks the pc-q35-8.2 machine. fetch-bootloader downloads Limine 12.8.0 and verifies SHA-256 without extracting or running the archive. .cache is excluded from the repository. If a version is no longer available through apt, installation fails: update the baseline with review/evidence rather than silently substituting latest. Ubuntu transitive dependencies and the runner image are not pinned by digest; this baseline does not promise hermetic rebuilds or binary identity.

The FAT32/UEFI image can already be built and booted. [Boot commands and limits](BOOT.md) cover individual execution and tests for success, panic, hangs and invalid arguments.

#33 adds [exceptions, time and waits](INTERRUPTS.md), with pure contracts tested on the host and IRQ, CPU fault and double-fault tests inside the guest. It requires no new crates or nightly.

#9 adds [memory and protections](MEMORY.md): frame bitmaps in the pure library and x86_64 page tables in the binary. The guest suite tests real exhaustion, recovery, independent spaces and five identified page faults.

#10 adds [ELF loading, lifecycle and ring 3 processes](PROCESSES.md). Validation and scheduling reside in the library; user memory and CPU transitions stay in architecture modules. Negative process cases are contained within the `ok` scenario.

#34 adds [IPC and handles](IPC.md), the original `rustic-abi` crate and validated buffer copying. The workspace and executor include `crates/`; the direct build fingerprint includes its code and manifests. External dependency versions remain unchanged.

## CI and negative testing

.github/workflows/check.yml runs the same cargo xtask check on ubuntu-24.04 for push and pull_request. Actions are pinned by SHA, the token has contents:read permissions, checkout does not persist credentials, and no project secrets are supplied. Toolchain versions, logs and the no_std library are retained as artifacts for 14 days. A separate job installs pinned QEMU/OVMF, tests the runner and boots all eighteen scenarios, preserving images and evidence.

Next, tools/check-failure.sh introduces a deliberately failing test and requires cargo xtask check to reject it. The check identifies the expected marker so a compilation or tooling error cannot count as evidence. Run it only in a disposable checkout: it formats and temporarily adds the fixture, removing it on exit.

```sh
bash tools/check-failure.sh
```

The same CI user could modify PR code; the workflow limits permissions and supplies no publishing credentials. The [#21 executor](EXECUTOR.md) adds builds and VMs in separate containers, resource/network restrictions, cancellation and manifests. Its CI job also checks real failures and clean repetition.

## Native application SDK

#11 adds [the native SDK and manifest](SDK.md). Run `python3 tools/application.py` to build the independent ELF and binary manifest. `cargo xtask check` requires Python 3.11+ (the reference uses Python 3.12), compiles/lints the guest application and passes its artifact directory explicitly when checking the kernel acceptance image. It still does not need QEMU or bootloader downloads. Host builds do not expose the SDK instruction boundary.

The image builder and isolated worker compile the application before the kernel with the `sdk-test` feature. Direct source fingerprints cover application sources, linker script, descriptor and the host manifest encoder. The sandbox exports bounded ELF/manifest artifacts with hashes alongside the containing kernel.

#35 adds [block storage](BLOCK.md), with PCI I/O, DMA, queue mechanics and request validation in separate modules. No toolchain or Cargo dependency is added. Full direct and isolated suites now contain 18 and 22 scenarios. Rebuild reviewed sandbox infrastructure before using the new modes.

## Logical service contracts

The [service contract guide](SERVICE-CONTRACTS.md) provides the commands for the separate Python 3.12 host validator and descriptor exporter. Install its exact hashed wheels into `.cache/contracts-venv`; no package is added to the kernel, SDK or sandbox image. CI checks 62 messages, nine exchanges and 14 message/descriptor tests and preserves their results. This is separate from the runner tests, finite operation model and actual guest acceptance.
