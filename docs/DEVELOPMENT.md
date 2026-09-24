<!-- SPDX-License-Identifier: Apache-2.0 -->

# Development and checks

## Environment

Verified baselines: Ubuntu 24.04 and 26.04 amd64 (including WSL2), Rust 1.98.1 and rustup 1.29.1. `tools/environment.toml` pins exact package versions and firmware hashes per release and `tools/environment.py` selects the one the host reports; another release is refused, not resolved to whatever apt offers. CI and the isolated executor stay on 24.04. On Windows, run the commands below inside Ubuntu. The checkout may live under /mnt/c; a checkout on the Linux filesystem can improve I/O performance.

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

Project-scoped Codex roles and the bounded delegation workflow are documented in
[Development orchestration](ORCHESTRATION.md). Start continuation work from
[Current work state](WORK-STATE.md). This optional host configuration adds no
requirement to build, boot, CI or run RusticOS manually.

The optional [JEV context-ranking pilot](JEV-CONTEXT.md) compares a local lexical
baseline with typed decisions through OpenRouter on explicitly selected excerpts.
It is a host experiment with no automatic CI or guest API calls.
The [failure diagnosis pilot](JEV-TRIAGE.md) adds explicit log-based advisories;
the [integration plan](JEV-PLAN.md) separates host work from future native services.

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

install installs the exact versions pinned for the host's Ubuntu release in tools/environment.toml; verify prints the selected baseline and rejects different packages or firmware hashes and checks the pc-q35-8.2 machine. fetch-bootloader downloads Limine 12.8.0 and verifies SHA-256 without extracting or running the archive. .cache is excluded from the repository. If a version is no longer available through apt, installation fails: add or update the release baseline with review/evidence rather than silently substituting latest. Ubuntu transitive dependencies and the runner image are not pinned by digest; this baseline does not promise hermetic rebuilds or binary identity.

The FAT32/UEFI image can already be built and booted. [Boot commands and limits](BOOT.md) cover individual execution and tests for success, panic, hangs and invalid arguments.

The selected release and its pins are recorded in image and measurement metadata. Rebuild images after changing the environment: the latency runner refuses stored images whose environment differs from the current selection. Evidence from different releases must retain its original provenance.

#33 adds [exceptions, time and waits](INTERRUPTS.md), with pure contracts tested on the host and IRQ, CPU fault and double-fault tests inside the guest. It requires no new crates or nightly.

#9 adds [memory and protections](MEMORY.md): frame bitmaps in the pure library and x86_64 page tables in the binary. The guest suite tests real exhaustion, recovery, independent spaces and five identified page faults.

#10 adds [ELF loading, lifecycle and ring 3 processes](PROCESSES.md). Validation and scheduling reside in the library; user memory and CPU transitions stay in architecture modules. Negative process cases are contained within the `ok` scenario.

#34 adds [IPC and handles](IPC.md), the original `rustic-abi` crate and validated buffer copying. The workspace and executor include `crates/`; the direct build fingerprint includes its code and manifests. External dependency versions remain unchanged.

## CI and negative testing

.github/workflows/check.yml runs the same cargo xtask check on ubuntu-24.04 for push and pull_request. Actions are pinned by SHA, the token has contents:read permissions, checkout does not persist credentials, and no project secrets are supplied. Toolchain versions, logs and the no_std library are retained as artifacts for 14 days. A separate job installs pinned QEMU/OVMF, tests the runner, boots all twenty-two scenarios, preserving images and evidence. The disposable V7 harnesses run as their own `v7` job (`python3 tools/boot.py v7`) beside that job (`python3 tools/boot.py test --skip-v7`), so neither outgrows its time cap; locally `python3 tools/boot.py test` still runs both in sequence. Ubuntu 24.04 remains the pinned CI reference; the local developer baseline on this machine is Ubuntu 26.04 and is checked against its own recorded package versions.

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

## v6 volume images

The v6 layout (#51) has an independent reader. `cargo test -p rustic-fs --test fs6_image`
provisions and migrates real volume images and exports the prefix each one uses, and

```sh
python3 tools/fs6_test.py
```

reads those images with `terminal_support/oracle6.py`, written from the format description
with Python's own CRC rather than from the Rust code, compares the migration output with
the v5 reader's view of the source image, and records the damaged images the reader
refuses. It needs only the pinned Rust toolchain, writes `artifacts/fs6/` and boots no
guest, so it runs in the workspace-check job. The same suite drives the host tool that
owns those images:

```sh
cargo run -p rustic-volume -- provision <image> <32-hex-lineage>
cargo run -p rustic-volume -- seed <image>
cargo run -p rustic-volume -- write <image> <parent-id> <name> <source-file>
cargo run -p rustic-volume -- migrate <image> <32-hex-lineage>
cargo run -p rustic-volume -- report <image>
```

`seed` writes a small v5 experiment volume, never the owner's terminal volume, and
`migrate` is the deliberate one-way upgrade. Host agreement is not guest execution: no
guest mounts a v6 volume yet.

## v7 volume images

The v7 layout has its own independent reader, `terminal_support/oracle7.py`,
written from [WORKSPACE-FORMAT7.md](WORKSPACE-FORMAT7.md) with Python's own CRC
and SHA-256. `cargo test -p rustic-fs --test fs7_image` builds full-size sparse
images through the public `Volume7` owner (writing them to `RUSTIC_FS7_EXPORT`
when set), and

```sh
python3 tools/fs7_test.py [--sweep N] [--seed S]
```

reads them plus a `seed7` fixture with the reader, compares each with
`rustic-volume report7`, migrates the three `seed5-history` v5 sources with
`migrate7` and compares each result with the independent v5 reader's view of
its unchanged source, and requires the reader and the Rust mount to give the
same accept/refuse verdict on 17 named damaged copies and on a deterministic
sweep of resealed field perturbations (default 60 per image; the run fails if
either verdict occurs fewer than one time in ten). It needs only the
pinned Rust toolchain, boots no guest, writes `artifacts/fs7/` and runs in the
`check` job of the "Workspace checks" workflow (`.github/workflows/check.yml`). The reader's parsing rules have unit tests in
`tools/tests/test_oracle7.py`.

The explicitly selected V7 read-only guest fixture uses a fresh disposable
volume and the measured largest shipped native artifact. Run
`python3 tools/v7_read_test.py` to build the host provisioner and UEFI image,
read the full `file-server.elf` and manifest over the real guest UART in two
boots, restart the file service between reads, stage the pinned pair as a
dormant child (and refuse stale pins) before and after that restart, and verify
the provisioned volume is unchanged. It writes no volume image to the tracked workspace or to
`artifacts/terminal/data.raw`; evidence is stored in
`artifacts/boot/terminal-v7/`. The harness uses only the read protocol, although
the shell now holds a V7 admission-profile grant (tracked writes and staged
admissions). The
temporary QEMU data backend is not opened read-only because V7 mount requires an
initial flush, which this host's read-only QEMU backend rejects; before/after
volume hashes independently check for mutation.

`python3 tools/v7_launch_test.py` builds that image once, copies it aside and
boots the same bytes twice. It builds two tagged utility variants
(`RUSTIC_UTILITY_TAG=1` and `2`) in separate cargo invocations under
`target/v7-launch/`, never into `target/native`, seeds each into its own fresh
V7 volume, and in each boot stages the pair, checks the start refusals, starts
the child control-only with `start-staged PID exit`, reads the variant tag from
`permissions PID` and reaps exit code 7. It records the image, kernel and
variant digests and the unchanged volume digests in
`artifacts/boot/terminal-v7-launch/`. `python3 tools/boot.py test` runs it after
`tools/v7_read_test.py`.

`python3 tools/v7_corrupt_test.py` builds the `terminal-v7` image and boots it
twice on damaged temporary copies of a fresh V7 volume: one with a flipped ELF
payload byte, which mount must refuse without a panic while owner control stays
usable, and one with a torn newest header copy, which must mount the older
generation and read its exact bytes. Evidence is stored in
`artifacts/boot/terminal-v7-corrupt/`; `python3 tools/boot.py test` runs it after
`tools/v7_launch_test.py`.

`python3 tools/v7_write_test.py` builds the same image, seeds a fresh temporary
volume with `rustic-volume seed7 ... --scratch` and boots it twice. Boot 1
streams six deterministic patterns (513 B to 512 KiB) into `scratch.bin` with
`replace-pattern-v7` until the eight-record budget is full, and checks a stale
version and the following `Full`. Before the last write it cuts a 512 KiB write
after 31 sectors acknowledged (inferred from the acknowledged bytes) and has
the owner revoke the shell's binding, which
must answer `Closed` on the old endpoint and `NoTransfer` on the new binding.
Boot 2 replays one write exactly, checks two mismatched retries and looks up a
small and a large write by operation ID and by retry key, which must print the
commit-time receipt lines. After each boot `oracle7` must find retained records
that match every printed receipt and none for the revoked key, and the volume
digest must not change across boot 2. Evidence, including guest ticks per write
size, per lookup and for the revocation, is stored in
`artifacts/boot/terminal-v7-write/`; `python3 tools/boot.py test` runs it after
`tools/v7_corrupt_test.py`. The receipt, lookup and cut parsing and the record
matching have unit tests in `tools/tests/test_v7_write.py`. See [V7 tracked writes](FILES-V7-WRITES.md).

`python3 tools/v7_retention_test.py` seeds another fresh temporary volume the
same way and boots it twice. It runs three cycles, two in boot 1 and one after
the reboot, that fill the eight-record budget with 4 KiB `replace-pattern-v7`
writes, check `Full`, run the owner's `maintain-v7`, write in the new epoch and
check that an exact retry, a fresh write and a retry lookup naming the old
epoch are `ExpiredEpoch` and a reclaimed operation ID is `OutcomeUnknown`. In
the first cycle the owner first asks for maintenance while an exact retry holds
a transfer (`replace-pattern-v7 ... hold 40`), which must be `Busy` with the
image digest unchanged. `oracle7` checks the image before and after every
maintenance while the guest is idle, and after each shutdown: epoch, records,
freed snapshot sectors and unchanged live files. Evidence, including the guest
ticks of every maintenance, is stored in
`artifacts/boot/terminal-v7-retention/`; `python3 tools/boot.py test` runs it
after `tools/v7_write_test.py`. The report parsing and the oracle comparison
have unit tests in `tools/tests/test_v7_retention.py`. See
[owner retention maintenance](FILES-V7-WRITES.md#owner-retention-maintenance).

`python3 tools/v7_faults_test.py` builds the same image and interrupts an 8 KiB
`replace-pattern-v7` write at twelve device events and `maintain-v7` at nine,
each on a temporary copy of a base volume with one QEMU `blkdebug` EIO
(`recovery_faults.rules`). Every operation must report `Uncertain`. After
`restart files` and after a clean reboot, lookups, range reads and an exact
retry (or a repeated maintenance) must agree with the generation that
`oracle7` finds; only the final-flush cuts end in the new generation. A
mount-only boot must leave the image digest unchanged. The same run records
`mem` at every idle, fenced, restarted and post-operation point, requires all
of them to equal the idle baseline, times owner `INFO` round trips during two
512 KiB writes (`probe 64`) and records the revocation ticks. It takes 44
boots, about two and a half minutes of VM time on the reference machine.
Evidence is stored in `artifacts/boot/terminal-v7-faults/`;
`python3 tools/boot.py test` runs it after `tools/v7_retention_test.py`. The
event plan, the generation classification and the parsers have unit tests in
`tools/tests/test_v7_faults.py`. See
[interrupted publication](FILES-V7-WRITES.md#interrupted-publication).

`python3 tools/v7_admission_test.py` builds the same image, seeds another fresh
temporary volume with `seed7 --scratch` and boots it twice. Boot 1 admits a
64 KiB pattern with `admit-pattern-v7`, repeats it exactly, queries it by retry
identity, ID and observation v2, and has the owner's `maintain-v7` refused
with `Busy`, all with an unchanged image digest. Boot 2 executes the admission
and looks up its completion receipt by ID and retry key. A second admission is
overtaken by a tracked write: execution is `Version`, then an explicit cancel
records the `requested` cause. The final maintenance succeeds. `oracle7` checks
the image while the guest is idle after every phase and after each shutdown.
Evidence is stored in `artifacts/boot/terminal-v7-admission/`, and
`python3 tools/boot.py test` runs it after `tools/v7_faults_test.py`. The output
parsers and the oracle comparison have unit tests in
`tools/tests/test_v7_admission.py`. See [V7 staged admissions](FILES-V7-ADMISSIONS.md).

`python3 tools/v7_authority_test.py` builds the same image, seeds another fresh
temporary volume and boots it twice. Boot 1 revokes the shell's binding while
the first publication write of an EXECUTE is held by the kernel's completion
hold (`execute-admission-v7 ID revoke 0 200`): the admission must end
`cancelled` with cause `authority_lost`, the file unchanged and exactly one new
generation. The same diagnostic during an ACCEPT must leave no record or
generation, after which the same key admits and executes normally. Boot 2
re-reads the cancelled status and cause. `oracle7` checks the image after
every phase and after each shutdown. Evidence is stored in
`artifacts/boot/terminal-v7-authority/`, and `python3 tools/boot.py test` runs
it after `tools/v7_admission_test.py`. The output parser has unit tests in
`tools/tests/test_v7_authority.py`. See
[owner control during a publication](FILES-V7-ADMISSIONS.md#owner-control-during-a-publication).

The deliberate v5 -> v7 data migration has its own host commands:

```sh
cargo run -p rustic-volume -- seed5-history <v5-image> <32-hex-lineage> <receipts|admissions|completed>
cargo run -p rustic-volume -- migrate7 <v5-image> <v7-target> <32-hex-lineage>
```

`seed5-history` exclusively creates a disposable v5 source with two-record
scoped history, and `migrate7` reads an exact legacy-size v5 image, creates the
target exclusively, removes it on any failure and prints `report7` with the
source SHA-256 before and after; see
[deliberate data migration](WORKSPACE-FORMAT7.md#deliberate-data-migration-of-a-disposable-image).
Neither accepts or touches `artifacts/terminal/data.raw`.
`python3 tools/v7_migration_test.py` seeds and migrates all three sets in a
temporary directory and boots the `terminal-v7` image four times on the
migrated images: receipts (lookup, exact replay, mismatched retry, hidden
subject-1 record), admissions (Busy lookup and maintenance, requested cause,
execution, then a reboot with identical output) and an executed admission
(identical replays). `oracle7` checks every image before, during and after the
boots, and the v5 source digests must not change. Evidence is stored in
`artifacts/boot/terminal-v7-migration/`, and `python3 tools/boot.py test` runs
it after `tools/v7_authority_test.py`. The host-side comparison has unit tests
in `tools/tests/test_v7_migration.py`. It does not exercise executable
rollback, which stays in the launch harness (#52). See
[migrated history in the guest](FILES-V7-ADMISSIONS.md#migrated-history-in-the-guest).

The reviewed sandbox image builds the reference `rustic-volume` (`cargo build -p rustic-volume`)
so an isolated `block-user` case provisions its workspace volume with the reference writer
rather than with the candidate under test.

The `block-user` mode mounts a host-provisioned v6 workspace volume and reads a
16 KiB artifact through the real block device in 4 KiB ranges; its documented
floor is 90 s (`MODE_FLOOR` in `tools/boot_support/runner.py`), while every other
mode keeps the caller's timeout.

## Delayed-device regression

The separate delayed-device regression runs after the direct boot suite in CI:

```sh
python3 tools/latency_test.py --image artifacts/boot/recovery-test/rustic-os.img --output artifacts/latency/new-run
```

Use a new output directory; omit `--image` to build the current recovery image. It suspends an actual FLUSH for 0.6 seconds (successful bounded replacement) and six seconds (500-tick timeout, uncertain result and explicit recovery). The guest still uses its real VirtIO driver. A dedicated second QEMU exports the test disk through a private Unix NBD socket so the backend can resume independently of a guest device reset. This is a separate host fault topology, not a performance sample or a change to the ordinary R0 disk path. See [the block guide](BLOCK.md#delayed-device-regression) for evidence and limits.

## Repeated native measurements

The [measurement guide](MEASUREMENTS.md) defines the separate #20 protocol, metric boundaries, environment identity and regression decision. Run `python3 tools/measure.py verify --host-label local-wsl-r0 --samples 5 --output artifacts/measurements/new-run` after activating the pinned Rust environment. The output directory must be new. Each successful check performs 36 actual VM boots: three batches, each with one excluded warmup and five measured repetitions, with two VMs per repetition. The CI measurements job uses its own same-job baseline; it does not compare GitHub timings to WSL.

## Completed workspace operations

The [native lifecycle negotiation](FILE-NEGOTIATION.md) extends the terminal
mission with mounted support, exact contract digests, separate rights and current
responder checks across service restart/reboot. Run `negotiation-native` against
the resulting `artifacts/terminal-test/terminal.json`; the current host suites
contain 81 contract tests and 180 runner tests. The direct/isolated VM inventories
and evidence budgets are unchanged. Rebuild reviewed sandbox infrastructure
after this harness change. Counts in earlier increment records are historical.

The selected-client mission also retains its own admission ID, rejects repeated
mutation steps, preserves a human edit made between admission and scheduling, and
rejects readback from an identical later write. The terminal/result JSON summaries
are compact without dropping fields or raising their 64 KiB collector limits.

The [service-v2 lifecycle guide](FILE-LIFECYCLE.md) documents the current typed
inspection/minimal cancellation bindings and `lifecycle-check`, `lifecycle-export`
and `lifecycle-native` commands. That delivery passed 80 contract tests and
174 runner tests; see the current counts above.
Its native evidence extends the existing terminal mission, with no extra recovery
groups, VM modes or resource/export limits. CI validates the new terminal report.
Rebuild reviewed sandbox infrastructure after this harness change.

The [operation guide](FILE-OPERATIONS.md) defines the bounded files.replace/operations.get profile, explicit format migration, current authority and historical lookup. With the pinned validator environment installed:

```sh
.cache/contracts-venv/bin/python -m tools.contracts operations-check --output artifacts/operations-host.json
python3 tools/boot.py run --mode recovery-test --timeout 60
.cache/contracts-venv/bin/python -m tools.contracts operations-native \
  --evidence artifacts/boot/recovery-test/recovery.json --output artifacts/operations-native.json
.cache/contracts-venv/bin/python -m tools.contracts activity-check --output artifacts/activity-host.json
.cache/contracts-venv/bin/python -m tools.contracts capabilities-check --output artifacts/capabilities-host.json
.cache/contracts-venv/bin/python -m tools.contracts capabilities-native \
  --evidence artifacts/terminal-test/terminal.json --output artifacts/capabilities-native.json
.cache/contracts-venv/bin/python -m tools.contracts activity-native \
  --evidence artifacts/boot/recovery-test/recovery.json --output artifacts/activity-native.json
```

The current combined recovery inventory is 49 groups across 98 VM boots, including six workspace-operation groups, four [owner-control groups](FILE-CONTROL.md), four [public-admission groups](FILE-ADMISSION-API.md), six [live execution control groups](FILE-ACTIVITY.md), one saturated-execution group, one discarded-acknowledgement group, four [scheduled-execution groups](FILE-SCHEDULING.md), nine [scheduled failure groups](FILE-SCHEDULING-FAILURES.md) and five [scheduling authority groups](FILE-SCHEDULING-AUTHORITY.md). The common boot gate requires 31 held-I/O observations, 91 activity replies (including queued activity), 13 profile-1 and four profile-2 [coherent observations](FILE-OBSERVATION.md), and 17 expected uncertainty responses. The contract suite has 75 tests; the runner suite has 172. The completed-operation validator also checks the scheduling/failure groups, independent file versions/digests and matching receipts. Ten payload vectors produce 30 shared replacement/lookup exchanges; host fixture and native evidence are identified separately. Live activity and stop requests are a native profile with a checked [service-v1 correspondence](FILE-ACTIVITY.md), not yet full service-v1 lifecycle conformance. The full direct suite still has 22 scenarios, and the isolated suite 26. Rebuild the reviewed executor infrastructure with `python3 tools/sandbox.py prepare` before testing a committed candidate with the new drivers/fixtures. Earlier counts in dated or explicitly historical increment records describe those older revisions.

The [publication mechanics increment](FILE-PUBLICATION.md) extends the existing two-VM `block-user` scenario with a second native application per boot. It covers 16 early cancellation boundaries, late cancellation/settlement, restart replay and an independent disk oracle. It does not add a boot mode or expose the general service cancellation API. Rebuild the reviewed sandbox infrastructure before using this extended fixture against a committed candidate.

The [retained prevention increment](FILE-PREVENTION.md) adds explicit format-5 migration and cause retention. The terminal mission checks legacy unknown-cause preservation, requested cancellation and a scheduled version conflict across reboot. The `block-user` terminal volume checks a retained authority-loss cause and a late committed result without prevention; its pending volume remains format 4. Both existing VM inventories are unchanged. Rebuild reviewed sandbox infrastructure after this evidence-contract change.

The [cause-aware observation profile](FILE-OBSERVATION.md) adds native profile 2
without changing profile 1. The recovery gate now also requires four profile-2
replies (committed with no cause and legacy Cancelled with Unknown, across reboot).
The terminal mission compares both profiles and manual/deterministic clients for
legacy and format-5 causes, live pending work, read-only restart and empty denials.
Its exported prevention report is checked by `tools/terminal_support/prevention_report.py`.
The current contract suite has 75 tests and the runner suite 172; the VM inventories
remain 49 recovery groups / 98 boots, 22 direct scenarios and 26 isolated scenarios.
Rebuild reviewed sandbox infrastructure after this harness change.

The [durable admission increment](FILE-ADMISSION.md) adds two sequential native test applications per `block-user` boot, preserving each application's existing event and memory limits. Two further disposable volumes retain admitted, cancelled and committed records across both VMs. The host oracle requires unchanged volume hashes on replay and checks that admitted work has not executed. Use `python3 tools/boot.py run --mode block-user --timeout 45`; the complete inventories remain 22 direct and 26 isolated scenarios. Rebuild the reviewed sandbox infrastructure after this harness change.

The isolated suite selects 45 seconds per `block-user` VM to match direct CI after expanding that VM from two to four sequential application fixtures. The first local expanded boot took 38.302 seconds; the replay boot took 6.755. Other scenarios' timeout selections and every application's 4,096-event limit are unchanged. This is a test-workload budget, not an increase to guest process, memory or device quotas.

The [admission control integration](FILE-ADMISSION-CONTROL.md) reuses those four applications and two disposable admission volumes to run the service policy with real pollable block commands. The evidence now requires service-control and fresh-authority checks; both VM count and timeout remain unchanged. Rebuild reviewed sandbox infrastructure after this evidence-contract change. New native service callbacks are deterministic fixtures; the production private owner IPC is exercised separately by the existing recovery scenario.
