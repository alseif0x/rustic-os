<!-- SPDX-License-Identifier: Apache-2.0 -->

# Current work state

Updated 2026-10-03. Replace this checkpoint rather than appending conversation history.

## Active order and authorization

The owner selected **complete unification on V7** and autonomous continuation.
The requested stop after queue/cancellation `38cc2ab` was fulfilled; the later
“adelante” explicitly resumed work. Port existing terminal/application behavior,
verify authority/recovery, then retire RUSTFS1. Order: storage unification,
usable tasks on fresh volumes, external MCP over COM2. Do not introduce public
features, protocols, formats, versions, migrations, upgrades or rollback paths.
See [STORAGE-POLICY.md](STORAGE-POLICY.md).

Branch `v7-single-format`; native consumers and owner maintenance were committed
and pushed as `344b6d7` and `52fca7a`; this default-manual increment follows them.
Earlier commits: `38cc2ab`, `9b1e937`, `c693cea`, `58bfe10`, `b5f058d` and `5f9bc70`, based on
`main` at `097b4df`. No final-head CI or merge is claimed. No open PR exists for
this branch. #51 is closed; #52 remains open. Terminal parity traces to #22
and requirement R02; this increment does not close that broader issue.

Implementation decisions, commits, pushes, PRs and merges are authorized.
Observe every authoritative CI job passing on the exact final PR head before
merging; no early auto-merge. Never experiment on `artifacts/terminal/data.raw`.
Preserve unrelated owner changes, especially `LICENSE`. Documentation and PRs
remain English. Root owns shared builds/VMs/git; one writer per area, at most
two children. Use the project orchestration skill.

## Implemented V7 contracts to preserve

- Ordinary namespace/read and BEGIN/CHUNK/COMMIT/ABORT preserve full u32 lengths,
  scoped visible listing and copy-on-write without allocating retry records.
  Plain and streamed candidates share two slots. The explicit workspace shell
  has scope 4, rights 15, subject 2; scope zero is the whole mounted volume.
- Root/helper authority, one disjoint companion file, rights subsets, expiry,
  stale-root-safe revocation and generation changes are ported. Revocation
  clears service reply caches and closes affected endpoints without closing a
  fresh reused binding. ACKs report mask, released stages, fencing and settled
  sequence; a fenced V7 header has unavailable sequence zero.
- Both operation/admission profiles use the same eight Record7 slots. Profile 1
  is bounded to 1024 bytes; profile 2 to 512 KiB. Tracked publication has 103
  commands/barriers; pre-header authority loss prevents, later loss settles.
- The owner explicitly approved shared V7 retry history for the original
  RECOVERY/TRACK_BEGIN/RECEIPT commands. Fresh legacy writes use canonical
  top-level ancestry, exact modern direct records replay, ambiguous flat keys
  refuse. Modern retries remain workspace-qualified. Do not ask again.
- CAPABILITIES reports actual mounted bounds; DESCRIBE reports eight retained
  admissions, two volatile FIFO tickets and one active publication. Live
  observation/stop uses captured scope proofs while publication borrows the
  volume. Direct EXECUTE of a ticketed admission is Busy after authorization.
  Requested stops drain pending I/O; AuthorityLost has priority; later work
  settles. Restart or uncertainty discards tickets, never resumes admissions.
- Storage-staged application launch remains control-only. Embedded file-access
  roles now use V7 grants as described below.

Earlier evidence, limitations and reviewer corrections are preserved in
[the port archive](WORK-STATE-V7-PORT-2026-10-02.md). Foundation and branch
consolidation history is in [the earlier archive](WORK-STATE-ARCHIVE-2026-10-02.md).
See [FILES-V7-TERMINAL.md](FILES-V7-TERMINAL.md) for current contract decisions.

## Default-manual V7 increment

`terminal` and `terminal-init` now mount host-prepared V7. The supervisor stores
shell policy separately from file profile: Manual is scope zero, rights 15,
subject 1; the explicit `terminal-v7` workspace fixture remains scope 4, rights
15, subject 2. Mount, restart, adoption and shell rebind use the selected policy.
The private owner stays read-only, scope zero, subject zero. V7 journal guards
continue reserving both subjects 1 and 2. Kernel changes only select bootstrap;
no new unsafe, dependency, format, packet or selectable mode spelling was added.

The manual launcher exclusively provisions fresh V7 with the existing owner
policy under a separate file lock, validates the exact prefix, and extends only
that fresh regular single-link file to the R0 sparse 4 GiB device with fsync.
The filesystem retains its frozen 131,282-sector prefix (64 MiB payload); the
remaining physical sectors do not enlarge it. Existing data must be a mountable
V7 prefix on a dedicated 4 GiB file. Missing/linked/special/wrong-size/corrupt/
legacy media and lock contention refuse; no conversion or repair is performed.
`report7` and exact-size disposable fixture tools remain strict. The default
CLI no longer exposes the legacy recovery-upgrade flag. The owner disk was never
used for tests.

Existing `terminal-test`/`recovery-test` spellings select a private legacy
acceptance branch. That service mounts first and initializes only after Empty;
its initializer also rejects nonzero reserved sectors. Second boots reuse the
same fixture bytes, preserving populated volumes. Normal tasks-owner, capacity
and measurement acceptance now use the existing ordinary recovery fixture;
their implementations remain legacy pending the remaining port. Shell help and
task-enable output report the actual mounted behavior/bounds.

Validation on Ubuntu 26.04 / pinned QEMU 10.2.1:

- `cargo xtask check`: 762 Rust tests in 100 suites, formatting, host/guest
  Clippy and builds pass. Python runner suite: 342 tests pass, including dedicated
  device refusal/locking and mismatched mode/build/kernel provenance refusals.
- `v7_manual_test.py`: two boots (`terminal-init`, `terminal`) with verified
  identical kernel/build, whole-volume navigation, system write refusal,
  subject-1 lost-reply receipt, shell task edit and cleared `/config/tasks-intent`,
  cut rebind reporting Closed/NoTransfer, restart preserving authority, same
  receipt after reboot and idle tasks recovery. The entire 4 GiB device stays
  unchanged after boot 2. Build `d9c7997a2d511400`, 20.974 seconds.
- `v7_consumers_test.py`: three boots, scoped native consumers, tasks owner,
  invalid policy refusal, group revocation, fresh grants and lost admission
  reconciliation pass on the same ordinary build, 10.214 seconds.
- `v7_owner_test.py`: two boots, read-only confirmations, shared rotation,
  refusal hashes, finite/indefinite stalled-service recovery and fresh grants
  pass on that same build, 15.774 seconds.
- `terminal_test.py`: 2,985 commands across two populated legacy fixture boots
  pass on acceptance build `3432032777cf5282`, 205.705 seconds. It validates the
  preserved legacy behavior, not default V7 parity for every former scenario.

Four native harnesses establish nine boots on final Rust sources. File-server
ELF is 473,496 bytes, below 524,288. Both new owner/manual harnesses are in the
V7 CI inventory. Evidence: `artifacts/boot/terminal-v7-{manual,consumers,owner}/`
and `artifacts/terminal-test/result.json`; logs
`/tmp/rustic-v7-manual-{check,python,native,consumers,owner,legacy}.log`.

Luna max implemented the bounded native policy/bootstrap changes under the
existing documented unavailable-provider fallback. Root owns integration and
all builds/VMs. Independent source inspection found a mode label overwritten
by package provenance; root filtered the metadata, added identity refusal tests,
and reran the manual harness. Explicit Astra low final independent review
completed through the inspection agent's single review child after direct root
review delegation hit its thread limit. It found no material issue in the final
diff, module/authority/persistence boundaries or inspected evidence. No new unsafe
or dependency inversion. Locks coordinate cooperating launchers; no hostile host
path-race protection is claimed. No quota savings, final-head CI or merge is
claimed.

Completed native-consumer (`344b6d7`) and owner-maintenance (`52fca7a`) evidence
is preserved in [the port archive](WORK-STATE-V7-PORT-2026-10-02.md).

## Next acceptance target

Default manual behavior is V7, but complete legacy-format retirement is pending.
Port the remaining recovery, block-probe, host/measurement and native acceptance
consumers with their existing behaviors, then retire RUSTFS1 after parity has
native evidence. Do not delete tests or claim one format prematurely. After
complete parity, open the PR and wait for every authoritative final-head CI job before merging to main. #22/#52
remain open; tasks usability and COM2 follow the storage phase.
