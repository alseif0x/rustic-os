<!-- SPDX-License-Identifier: Apache-2.0 -->

# Current work state

Updated 2026-10-02. Replace this checkpoint rather than appending conversation history.

## Active order and authorization

The owner selected **complete unification on V7** and autonomous continuation.
The requested stop after queue/cancellation `38cc2ab` was fulfilled; the later
“adelante” explicitly resumed work. Port existing terminal/application behavior,
verify authority/recovery, then retire RUSTFS1. Order: storage unification,
usable tasks on fresh volumes, external MCP over COM2. Do not introduce public
features, protocols, formats, versions, migrations, upgrades or rollback paths.
See [STORAGE-POLICY.md](STORAGE-POLICY.md).

Branch `v7-single-format`; this checkpoint accompanies native consumers after
`38cc2ab`, `9b1e937`, `c693cea`, `58bfe10`, `b5f058d` and `5f9bc70`, based on
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

## Native-consumer increment

The supervisor's private V7 client now has read-only whole-volume scope with
subject zero, allowing it to read `/config/owner-policy` and staged artifacts.
V7 policy loading never creates or repairs the file. Missing/malformed policy
leaves mounting and the workspace shell usable but disables file-access child
launches. Legacy policy initialization is preserved until format retirement.
`rustic-volume provision7 IMAGE LINEAGE` exclusively creates fresh V7 media with
the existing policy, using an ordinary replacement and no retry records.
Existing `seed7` behavior and workspace shell authority remain unchanged.

Native read/probe/session/helper/tasks roles no longer fail the supervisor's
V5-only readiness gate. Tasks owners receive two disjoint files and use the
journal object as subject, with subjects 1 and 2 reserved. The shell excludes
its `/config/tasks-intent` by authorized journal metadata; it does not require
parent-directory authority, so `/workspaces/tasks-intent` is valid. Owner-launched
lost-reply/admission actors retain subject 1, distinct from workspace shell 2.
Maintenance and shell rebind also wait for pending group takeover to settle.
No kernel change, unsafe, dependency, packet layout or disk format was added.

Validation on Ubuntu 26.04 / pinned QEMU 10.2.1:

- `cargo xtask check`: 760 Rust tests in 100 suites; formatting, host/guest
  Clippy and native builds passed on the final Rust implementation.
- Python runner suite: 335 tests passed. Both new harnesses are registered in
  `tools/boot.py` for the `v7` CI job.
- `v7_consumers_test.py`: three boots, including mountable same-length malformed
  policy refusal with unchanged volume; scoped probe; failed helper/second-scope
  grants without leaks; saturated client/helper reply queues, settled group
  revocation releasing a stage, old READ Closed and subsequent fresh grants;
  tasks apply using a separate journal; lost acceptance retained across reboot,
  foreign-subject lookup refused and explicit same-subject execution/replay.
  Oracle7 verifies live bytes and both retained outcomes. Ordinary build
  `74513684b93f693b`, 11.822 seconds.
- `v7_authority_test.py`: two boots on the same build, held ACCEPT/EXECUTE
  authority-loss prevention, unchanged targets and recovery pass; 10.995 seconds.
- `v7_tasks_recovery_test.py`: two boots on explicit `tasks-acceptance` build
  `62cdeed555f2f767`, 5.908 seconds. A lost replacement reply leaves a nonempty
  journal and exact committed receipt. A fresh client after reboot clears only
  the journal; target version/bytes and receipt remain unchanged, and idle
  repeat recovery changes no volume bytes.
- `terminal_test.py`: 2,983 commands across two boots pass, including the
  original policy initialization and reserved journal collision refusals, on
  acceptance build `62cdeed555f2f767`.
- `v7_read_test.py`: two boots, service restart, exact 485,136-byte ELF/manifest
  reads, stale pins, staging cancellation/cleanup and missing-policy consumer
  denial pass. The entire volume stays unchanged; build `74513684b93f693b`.

Evidence is in `artifacts/boot/terminal-v7-{consumers,authority,tasks-recovery}/result.json`.
Logs: `/tmp/rustic-v7-consumers-{check,python,native,authority,tasks-recovery,legacy,read}.log`.
The five native harnesses completed eleven boots on the final Rust sources.
File-server ELF remains 485,136 bytes, below 524,288.

Astra low final source/harness review found no material issue. Luna max completed
the bounded supervisor and recovery-harness work after the configured DeepSeek
route returned unavailable; no default model route changed. Root integrated,
fixed the shell's directory-authority dependency and harness error expectations,
and owns every build/VM run. No token/quota savings are claimed.

Limits: drain status alone is not closure evidence; subsequent READ Closed
checks establish it. Failed COMMIT transport reports Uncertain; settled owner
revocation and bytes establish prevention. The pending-takeover-before-admin-
submission guard is source-reviewed, not separately forced in a guest. Guest
power cuts model owned QEMU termination, not physical media durability. Full
final-head CI and complete default-terminal parity remain pending.

## Next acceptance target

Commit/push this verified increment, then continue with
existing owner maintenance selectors, fresh default-terminal provisioning and
whole-volume manual policy while retaining explicit workspace-scope fixtures.
Port remaining block-probe, host and native acceptance consumers; retire
RUSTFS1 only after parity has evidence. The default terminal still uses the
legacy Volume/Server. Do not delete its tests or claim one format prematurely.
After complete parity, open the PR and wait for every final-head CI job before
merging to main. Tasks usability and COM2 follow the storage phase.
