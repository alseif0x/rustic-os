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

rust-toolchain.toml selects this exact version; nightly is not required. Cargo.lock pins the Limine bindings and bitflags for the boot executable, plus the reviewed SHA-256 dependencies for native file-service/SDK range verification. The kernel does not depend on that hash implementation. See [the dependency inventory](dependencies.md) for versions, features and provenance. Checks use --locked. rust-lld produces a static ELF through kernel/linker.ld; the pure library remains separate from the executable.

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

.github/workflows/check.yml runs the same cargo xtask check on ubuntu-24.04 for push and pull_request. Actions are pinned by SHA, the token has contents:read permissions, checkout does not persist credentials, and no project secrets are supplied. Toolchain versions, logs and the no_std library are retained as artifacts for 14 days. A separate job installs pinned QEMU/OVMF, tests the runner and boots all twenty-two scenarios, preserving images and evidence.

Next, tools/check-failure.sh introduces a deliberately failing test and requires cargo xtask check to reject it. The check identifies the expected marker so a compilation or tooling error cannot count as evidence. Run it only in a disposable checkout: it formats and temporarily adds the fixture, removing it on exit.

```sh
bash tools/check-failure.sh
```

The same CI user could modify PR code; the workflow limits permissions and supplies no publishing credentials. The [#21 executor](EXECUTOR.md) adds builds and VMs in separate containers, resource/network restrictions, cancellation and manifests. Its CI job also checks real failures and clean repetition.

## Native application SDK

#11 adds [the native SDK and manifest](SDK.md). Run `python3 tools/application.py` to build both independent ELFs and their binary manifests. `cargo xtask check` requires Python 3.11+ (the reference uses Python 3.12), compiles/lints the guest application and passes its artifact directory explicitly when checking the kernel acceptance image. It still does not need QEMU or bootloader downloads. Host builds do not expose the SDK instruction boundary.

The image builder and isolated worker compile the application before the kernel with the `sdk-test` feature. Direct source fingerprints cover application sources, linker script, descriptor and the host manifest encoder. The sandbox exports bounded ELF/manifest artifacts with hashes alongside the containing kernel.

#35 adds [block storage](BLOCK.md), with PCI I/O, DMA, queue mechanics and request validation in separate modules. No toolchain or Cargo dependency is added. That increment supplied 18 direct and 22 isolated scenarios; #44 expands them to 20 and 24. Rebuild reviewed sandbox infrastructure before using the new modes.

## Logical service contracts

The [service contract guide](SERVICE-CONTRACTS.md) provides the commands for the separate Python 3.12 host validator and descriptor exporter. Install its exact hashed wheels into `.cache/contracts-venv`; those Python packages are not added to the kernel, SDK or sandbox image. CI checks message/exchange fixtures, descriptors and host conformance behavior and preserves their results. This is separate from the runner tests, finite operation model and actual guest acceptance.

The [native `files.read` guide](FILES-READ.md) separates pure codec/collector checks, a bounded immutable host fixture and actual terminal execution. After installing that validator environment, the focused procedures are:

```sh
cargo test -p rustic-abi -p rustic-sdk --test file_read --locked
.cache/contracts-venv/bin/python -m tools.contracts read-check --output artifacts/read-host.json
.cache/contracts-venv/bin/python -m unittest discover -s tools/contracts/tests -v
python3 tools/terminal_test.py
.cache/contracts-venv/bin/python -m tools.contracts read-native \
  --evidence artifacts/terminal-test/terminal.json --output artifacts/read-native.json
```

`read-native` validates shared range exchanges in the evidence produced by the preceding terminal run; it does not boot a guest itself. Both fixture backends cover one logical operation with the `complete_bounded_ranges` profile, which requires the full requested range up to EOF. The general service-v1 contract also permits shorter non-EOF progress. These commands do not establish the full eight-operation catalog, live discovery or MCP/function adapter conformance. Record the actual revision, backend and results for each run.

#44 adds [bounded user-mode disk access](BLOCK-ACCESS.md). The host builder/linter selects both `sdk-probe` and `block-probe`; both manifests and ELFs have separate hashes. Shared block codecs and pure ownership/queue tests run on the host, while two additional VM scenarios exercise actual copied sector calls, cancellation, process death and persistence. The regression inventory is 46 Rust and 30 runner tests, 20 direct VM scenarios and 24 isolated scenarios.

## Delayed-device regression

The separate delayed-device regression runs after the direct boot suite in CI:

```sh
python3 tools/latency_test.py --image artifacts/boot/recovery-test/rustic-os.img --output artifacts/latency/new-run
```

Use a new output directory; omit `--image` to build the current recovery image. It suspends an actual FLUSH for 0.6 seconds (successful bounded replacement) and six seconds (500-tick timeout, uncertain result and explicit recovery). The guest still uses its real VirtIO driver. A dedicated second QEMU exports the test disk through a private Unix NBD socket so the backend can resume independently of a guest device reset. This is a separate host fault topology, not a performance sample or a change to the ordinary R0 disk path. See [the block guide](BLOCK.md#delayed-device-regression) for evidence and limits.

## Repeated native measurements

The [measurement guide](MEASUREMENTS.md) defines the separate #20 protocol, metric boundaries, environment identity and regression decision. Run `python3 tools/measure.py verify --host-label local-wsl-r0 --samples 5 --output artifacts/measurements/new-run` after activating the pinned Rust environment. The output directory must be new. Each successful check performs 36 actual VM boots: three batches, each with one excluded warmup and five measured repetitions, with two VMs per repetition. The CI measurements job uses its own same-job baseline; it does not compare GitHub timings to WSL.

## Completed workspace operations

The [operation guide](FILE-OPERATIONS.md) defines the bounded files.replace/operations.get profile, explicit format migration, current authority and historical lookup. With the pinned validator environment installed:

```sh
.cache/contracts-venv/bin/python -m tools.contracts operations-check --output artifacts/operations-host.json
python3 tools/boot.py run --mode recovery-test --timeout 60
.cache/contracts-venv/bin/python -m tools.contracts operations-native \
  --evidence artifacts/boot/recovery-test/recovery.json --output artifacts/operations-native.json
```

The current combined recovery inventory is 19 groups across 38 VM boots, including six workspace-operation groups and four [owner-control groups](FILE-CONTROL.md). Ten payload vectors produce 30 shared replacement/lookup exchanges; host fixture and native evidence are identified separately. The full direct suite still has 22 scenarios, and the isolated suite 26. Rebuild the reviewed executor infrastructure with `python3 tools/sandbox.py prepare` before testing a committed candidate with the new drivers/fixtures. Earlier counts in dated or explicitly historical increment records describe those older revisions.

The [publication mechanics increment](FILE-PUBLICATION.md) extends the existing two-VM `block-user` scenario with a second native application per boot. It covers 16 early cancellation boundaries, late cancellation/settlement, restart replay and an independent disk oracle. It does not add a boot mode or expose the general service cancellation API. Rebuild the reviewed sandbox infrastructure before using this extended fixture against a committed candidate.
