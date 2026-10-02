<!-- SPDX-License-Identifier: Apache-2.0 -->

# Current work state

Updated 2026-10-02. Replace this checkpoint rather than appending conversation history.

## Active order and authorization

The owner selected **complete unification on V7** and autonomous continuation on
2026-10-02. Port existing terminal and application operations, verify their
authority/recovery contracts, then retire RUSTFS1. The three-phase order is:
storage unification, usable tasks on fresh volumes, external MCP over COM2.
No new public functionality, protocols, formats, versions, migrations, upgrades
or rollback paths are authorized in this order. See [STORAGE-POLICY.md](STORAGE-POLICY.md).

Branch: `v7-single-format`; committed authority port `58bfe10`, following
terminal port `b5f058d` and `5f9bc70`, based on `main` at `097b4df`.
There was no open PR when this port began. Issue #51 is closed; #52 remains open.
Namespace/terminal parity traces to #22 and requirement R02. An issue closure or
the V7 foundation does not establish completion of this unification.

The owner already authorizes implementation decisions, commits, pushes, PRs and
merges within scope. Observe every authoritative CI job passing on the exact
final PR head before merging; no early auto-merge. Never experiment on
`artifacts/terminal/data.raw`. Preserve unrelated owner changes, especially
`LICENSE`. Maintained documentation and PR text remain English.

## Completed port increment

V7 now handles existing namespace queries/mutations, ordinary READ and bounded
BEGIN/CHUNK/COMMIT/ABORT. Metadata preserves full u32 lengths; LIST uses visible
child ordinals without cursor wrap. The explicitly selected workspace terminal
sees only `/workspaces`. Grant peer/context/expiry and scoped rights are checked
before returning metadata or payload.

`Volume7::replace` uses copy-on-write and the existing publication barriers,
preserves retained snapshots and open-stage reservations, and allocates no retry
record. Plain transfers reuse the existing buffer pool, share a combined limit
of two with streamed stages, and clear on authority loss. An owner-bound ABORT
can release a candidate after another client removes its file. No new unsafe
code, dependency, kernel mechanism, packet layout or on-disk format was added.
See [FILES-V7-TERMINAL.md](FILES-V7-TERMINAL.md).

Verified on the port tree, Ubuntu 26.04 / pinned QEMU 10.2.1:

- `cargo xtask check`: 715 Rust tests, formatting, host/guest Clippy and builds.
- Focused service suites: 20 tests; untracked storage: 7 tests, including
  publication cuts, populated/full retention and stage reservations.
- Python runner suite: 335 tests passed after harness registration.
- `python3 tools/fs7_test.py`: four images, 17 damaged copies and 180 resealed
  perturbations agree with the independent reader.
- `python3 tools/v7_plain_test.py`: two boots, twenty ordinary writes, deletion
  without identity reuse, exact metadata/content after reboot and service
  restart, two populated retained snapshots unchanged. Evidence:
  `artifacts/boot/terminal-v7-plain/result.json`, build `8bdaa8c25d281d32`.
- `python3 tools/v7_read_test.py`: ordinary paths/listing/stat/cat across two
  boots and service restart; exact 417,384-byte ELF and manifest bytes; stale
  pins refused, restart during staging cleaned up, subsequent staging worked;
  complete volume digest unchanged. Evidence:
  `artifacts/boot/terminal-v7/result.json`, same source build ID.
- `python3 tools/terminal_test.py`: 2,983 completed commands over two boots;
  the existing default terminal and its independent persistent-file oracle
  passed. Evidence: `artifacts/terminal-test/terminal.json`.

Full logs are `/tmp/rustic-v7-terminal-{focused,check,plain,read,fs7}.log`.
Host disk fixtures model whole-sector writes/flush barriers; arbitrary torn
media or physical hardware durability is not established. Full final-head CI
is still pending. The default regression does not prove V7 parity for those
remaining consumers.

Astra low found a stranded candidate after deletion; root fixed it, added a
regression and obtained follow-up review with no remaining material findings.
The reviewer retracted its zero-retained-record concern after checking seed7,
which always retains the ELF/manifest. The configured DeepSeek route was
unavailable on this collaboration surface; GPT-6 Luna max completed the bounded
service work by the skill-authorized explicit fallback. No default route was
changed and no token/quota savings are claimed. Root owns shared builds/VMs.

## Next acceptance and remaining work

The authority increment ports root/helper lineage, attenuation,
one disjoint companion file, stale-root-safe group revocation, full owner scope
zero and exact rights subsets. Workspace fixtures must explicitly request node
4. Core `cargo test -p rustic-file-service --locked` passed 149 tests in 16
suites. Root additionally checks mixed plain/staged group cleanup,
helper detachment, extension refusals, expired-grant permanence and cached
receipt cleanup. Astra found cached receipt parts bypassing the live companion
scope check; root fixed and regression-tested it, and follow-up core review
found no remaining material issue. Native review also found no material defect
in group cleanup, fresh endpoint reuse, stale reply suppression or deferred ACKs.

Native admin routing now handles the existing derive, second scope and root
revocation commands. Loss closes all affected endpoints and clears queued
replies, preserving tokens used by fresh live bindings. Revocation reports the
affected mask, released candidates, fenced status and settled sequence; a
fenced V7 volume reports sequence zero as unavailable. The supervisor requires
an unfenced settlement with a valid shell mask before regranting.

Authority increment validation so far:

- `cargo xtask check`: 725 Rust tests in 96 suites, formatting, lints and native
  builds passed. Log: `/tmp/rustic-v7-authority-check.log`.
- `python3 tools/v7_authority_test.py`: two boots; held EXECUTE cancelled with
  authority-lost cause, unchanged target and one prevention generation; held
  ACCEPT published nothing and the same key subsequently admitted/executed.
  Evidence: `artifacts/boot/terminal-v7-authority/result.json`, source build
  `f9a4e8705d316f3b`.
- `python3 tools/v7_plain_test.py`: two boots, twenty writes and exact persistence
  through service restart, retained snapshots unchanged.
- `python3 tools/v7_read_test.py`: two boots and service restart; exact
  452,208-byte ELF (442 ranges) and manifest, workspace namespace filtering,
  stale pins and staging cancellation/cleanup passed; volume digest unchanged.
  Source build `f9a4e8705d316f3b`; evidence is in
  `artifacts/boot/terminal-v7/result.json`.
- `python3 tools/v7_write_test.py`: six streamed writes through 512 KiB,
  mid-transfer revocation closed the old endpoint and freed the stage, stale
  versions and Full refused, reboot replay and cold receipt lookups matched,
  independent oracle agreed and the second boot left the volume unchanged.
  Evidence: `artifacts/boot/terminal-v7-write/result.json`.

Both implementations used the explicitly recorded Luna max fallback; root
integrated and owns all tests. Full final-head CI remains pending. Group grant
semantics and mixed candidate cleanup have host evidence; group endpoint
revocation/regrant with a queued helper reply still requires guest evidence
when the legacy native consumers are ported. Current guest authority evidence
covers the shell binding and deferred publication ACK, not that helper case.

Authority is committed as `58bfe10`. The next increment ports existing
profile-1 tracked and admission codecs onto the same V7 owners. The shared
tracked candidate construction now supports pollable publication with exactly
the original 103 commands/barriers. Owner revocation before header submission
cancels a tracked effect; after submission it settles, reports uncertainty to
the old caller and permits recovery through a fresh authorized lookup.

Profile-1 increment validation:

- `cargo xtask check`: 738 Rust tests in 97 suites, formatting, lints and native
  builds passed (`/tmp/rustic-v7-profile1-check.log`).
- Seven focused storage cases cover identical media/replay, pre-/post-header
  stops, every publication failure, held-command draining, drop fencing,
  retained snapshots beside another open stage, and a final flush that becomes
  durable before reporting an error. Both receipt profiles have host coverage
  for revocation while a publication command is pending.
- Python runner suite: 335 tests passed.
- `python3 tools/v7_profile1_test.py`: two boots, exact profile-1 receipts,
  retries and cold lookups, admission execution/cancellation, and independent
  verification of live/retained bytes. A 1025-byte profile-2 receipt answers
  `Size` through profile 1 and remains readable through profile 2. Six of eight
  record slots are used. Evidence: `artifacts/boot/terminal-v7-profile1/result.json`,
  build `a6b9cbf7ef9c5115`.
- `python3 tools/v7_write_test.py`: existing six-size streamed-write, authority,
  Full, reboot and retained-lookup acceptance passed on the same build.
- `python3 tools/v7_faults_test.py`: 21 interrupted publications over 44
  boots (164 seconds), restart/reboot agree with the independent oracle. Only
  final-flush cuts select the new header; all 110 memory checkpoints retain
  the idle baseline. File-server static footprint is 112 of 256 allowed pages.
- `python3 tools/v7_authority_test.py`: held ACCEPT and EXECUTE owner-revocation
  cases pass again; cancelled execution survives reboot and cancelled acceptance
  leaves no record, allowing the same key to be used afterwards.

Native evidence uses source build `a6b9cbf7ef9c5115`; full logs are
`/tmp/rustic-v7-profile1-{native,write,faults,authority,python}.log`.

Luna max completed the bounded codec and harness work; root integrated owner
control and owns verification. Astra reviewed storage/service changes and the
new harness, with no remaining material findings. Final-head CI remains
pending. Existing consumers still need data/config scopes as well as workspaces.

The next mapped slice is mounted discovery. Do not advertise reviewed lifecycle
support until the live scheduling/cancellation path is implemented. Legacy
RECOVERY/TRACK_BEGIN/RECEIPT still execute on format 5 and must be accounted for:
its flat retry namespace is distinct from scoped operations, whereas V7 stores
workspace-qualified records without a legacy discriminator. Astra confirmed no existing root or identity can preserve that isolation.
The owner explicitly approved retaining the old framing with shared V7 retry
history on 2026-10-02: identical retries reuse the existing result, ambiguous
matches are refused. Preserve rights, size bounds and genuine workspace ancestry.
This approved projection is not exact historical namespace isolation and must
not introduce a format change.

Then port mounted capabilities/lifecycle negotiation, remaining
admission/scheduling/recovery consumers, safe fresh-volume provisioning, default
terminal selection and the block-probe/host/native acceptance fixtures. The
legacy Volume/Server still power the default terminal. Retain it until parity
has evidence; do not claim a single format or delete its tests prematurely.
Tasks usability and the COM2 bridge follow this storage phase.

Historical foundation, branch-consolidation, earlier acceptance evidence and
known limits remain in [the archived checkpoint](WORK-STATE-ARCHIVE-2026-10-02.md).
The format-removal baseline `5f9bc70` deleted V6 and conversion/rollback paths;
its 694-test/335-test and native evidence is in that archive. It did not remove
RUSTFS1 or complete terminal parity.
