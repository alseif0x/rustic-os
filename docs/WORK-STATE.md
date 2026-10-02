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

Branch: `v7-single-format`; parent revision `5f9bc70`, based on `main` at `097b4df`.
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

Port the existing owner-issued grant contract: root/helper lineage, attenuation,
one disjoint companion file, stale-root-safe group revocation and endpoint
cleanup during publication. Restore the existing terminal's owner-authorized
volume scope and rights combinations without promoting READ|WRITE to INSPECT.
Keep workspace fixtures explicitly scoped to node 4. This ports existing
authority, not a new delegation protocol; test each boundary before consumers
use it. Existing consumers need data/config scopes as well as workspaces.

Then port mounted capabilities/lifecycle negotiation, profile-1 tracked and
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
