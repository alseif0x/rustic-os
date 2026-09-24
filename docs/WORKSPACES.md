<!-- SPDX-License-Identifier: Apache-2.0 -->

# Application workspaces: measured limits and selected budget (#51)

Stage one of [#51](https://github.com/alseif0x/rustic-os/issues/51): measure the consuming workload, state the structural limits that a larger workspace has to change, and select an explicit acceptance budget. No capacity is implemented by this document.

## Measured consumers

The native application artifacts that a workspace must eventually carry, as built on this machine:

| Artifact | Bytes |
| --- | ---: |
| `file-server.elf` | 368,872 |
| `block-probe.elf` | 283,520 |
| `shell.elf` | 238,632 |
| `utility.elf` | 131,360 |
| `supervisor.elf` | 104,648 |
| `tasks.elf` | 53,192 |
| `sdk-probe.elf` | 26,280 |
| every `*.manifest` | 128 |

The largest shipped executable is **401,128 bytes** (`file-server.elf` after the V7 staged-admission service; it was 368,872 bytes after retention maintenance), and the manifest that identifies it is 128 bytes. Anything that installs or launches an application from a workspace (#52) therefore needs files of that order, not of the current order. Measurements were refreshed on 2026-09-24 from the `target/native` build that `python3 tools/v7_retention_test.py` produced for its `terminal-v7` image (a `tools/terminal_test.py` build leaves larger shell, utility and supervisor artifacts), during the V7 retention-maintenance increment. `file-server.elf` was 324,344 bytes on 2026-09-23 and 364,104 bytes after the V7 revocation and receipt lookups, and grew with the V7 tracked-write service, its lookups and maintenance; it still fits the 512 KiB (524,288-byte) V7 file limit. The 401,128-byte figure was measured on 2026-09-24 from the `target/native` build of `python3 tools/v7_admission_test.py`.

## Measured limits of the current volume

| Limit | Value | Where |
| --- | --- | --- |
| Objects per volume | 32 | `crates/fs/src/lib.rs:23` |
| File length, policy | 1,024 bytes | `crates/fs/src/lib.rs:24` (`MAX_FILE`) |
| File length, structure | 65,535 bytes | `Node.length: u16`, `crates/fs/src/namespace.rs:15` |
| Volume structures | 174 sectors (about 87 KiB) | `crates/fs/src/lib.rs:25` (`SECTORS`) |
| Data placement | `32 + slot * 4 + bank * 2` sectors | `crates/fs/src/storage.rs:14` |
| Bank header | `8 + bank * 5` | `crates/fs/src/storage.rs:11` |
| Retained operation records | 2 | the issue's provenance note and the transfer slots |
| On-disk format | v5 | `docs/FILES.md` |

Two structural consequences follow, and they are why raising `MAX_FILE` alone is not enough:

1. **A file is one bank.** One bank is two sectors, exactly 1,024 bytes, so the policy limit and the placement geometry agree today. A larger file needs a bank-per-file mapping (an extent list or a chain) that the format does not have.
2. **The length field is 16 bits.** Even with extent mapping, `Node.length: u16` caps any file at 65,535 bytes — 5.6 times below the largest artifact measured above.
3. **The fixed structures are small.** 174 sectors bound the whole volume's payload area; a 64 MiB workspace is not a constants change but a different allocation scheme.

Whole-file buffers are the other bound: the file service holds `Transfer.data: [u8; MAX_FILE]` for two slots and the filesystem layer keeps whole-file buffers, so raising the per-file limit multiplies RAM per slot by the same factor. A 1 MiB file with two slots is 2 MiB of RAM before any copy, which must be measured against the reference profile rather than assumed.

## Selected workload and budget

**Workload:** install and launch one native application artifact from a workspace, the consumer that #52 names. The current largest artifact is `file-server.elf` at 364,104 bytes (measured 2026-09-24) plus its 128-byte manifest.

**Budget, pinned to that consumer:**

- **512 KiB per V7 file** — covers the current largest shipped executable with about 61% headroom. V5/V6 limits remain frozen. A 512 KiB feature bit distinguishes this V7 profile from pre-release 256 KiB V7 images; old-format development fixtures must be reprovisioned.
- **256 objects** — eight times today's 32, enough for application generations plus configuration and task records.
- **64 MiB of file data per volume** — the issue's planning target, independent of the 256-object and 512 KiB per-file ceilings. Not all maximum-size files can coexist; total capacity is exhausted first. `python3 tools/v7_capacity_test.py` serves a volume with 200 objects and 130,972 of the 131,072 payload sectors allocated in the guest: objects 48, 72 and 200 read by reference, writes the 100 free sectors cannot hold are refused with `Full` with no transfer left open and the image unchanged, a smaller write then commits, and retention maintenance frees exactly the snapshot-only sectors so a refused write fits afterwards. The 256-entry object table is filled on the host with `add7`, which then refuses; the V7 service has no create request, so the guest cannot reach that limit ([storage exhaustion](FILES-V7-WRITES.md#storage-exhaustion-on-a-nearly-full-volume)). Mounting such a volume verifies every allocated payload sector and took 2,426 and 2,436 guest ticks in two runs (about 24 s under QEMU TCG); the supervisor's V7 start and restart jobs now have a measured 6,000-tick budget, because the ordinary 1,000-tick deadline timed out above roughly 22 MiB of allocated payload.
- **8 retained operation records** — four times today's two, so a client can recover a small window of unresolved outcomes instead of losing the third mutation to `Full`. Retention is bounded; v7 now has an explicit retry-epoch transition that refuses open admissions, and unresolved evidence is never silently dropped. In `mode=terminal-v7` the owner runs it as the `MAINTAIN_V7` job, refused with `Busy` while a transfer is open, and the guest harness shows three fill, maintain and write cycles across a reboot ([owner retention maintenance](FILES-V7-WRITES.md#owner-retention-maintenance)).

**Format follow-up:** at the time this budget was selected, the next design was described as a v6 layout with extents per file, a wider length field, typed total-data exhaustion and deliberate upgrade behavior from v5. The later [format-7 decision](WORKSPACE-FORMAT7.md) supersedes that target to persist scoped identity, immutable retry snapshots, staged admission and explicit retention without weakening the selected budget. The host-only `upgrade_v5_to_v7` converter copies into a distinct disposable target, preserving representable scoped evidence and refusing ambiguous history. An explicitly selected V7 service is wired behind `mode=terminal-v7`; v5 remains the default. The V7 service now also accepts streamed profile-2 tracked writes up to 512 KiB from the shell, and `python3 tools/v7_write_test.py` fills the eight-record budget, refuses the next write with `Full` and replays an exact retry after reboot with an identical receipt and no volume change ([V7 tracked writes](FILES-V7-WRITES.md)). `python3 tools/v7_read_test.py` now provisions a fresh temporary V7 volume and reads the complete `file-server.elf` plus manifest in two QEMU boots, including a service restart, while checking byte equality and an unchanged volume digest. The same harness now has the supervisor stage that pinned ELF/manifest pair as a dormant, never-started child and refuse stale pins ([storage-sourced dormant images](NATIVE-RUNTIME.md#storage-sourced-dormant-images-modeterminal-v7)). `python3 tools/v7_launch_test.py` then boots one frozen kernel image against two fresh V7 volumes holding separately built `utility` variants and starts each staged child with a control-only topology ([starting the staged child](NATIVE-RUNTIME.md#starting-the-staged-child-control-only)). `tools/terminal_support/oracle7.py` is an independent V7 reader written from the format document; `python3 tools/fs7_test.py` checks it against `report7` on host-written images with every retained-record state and records, per damaged copy, that it and the Rust mount agree on what is refused (see [independent reader and damaged input](WORKSPACE-FORMAT7.md#independent-reader-and-damaged-input)). `python3 tools/v7_corrupt_test.py` boots the V7 fixture on a volume with a flipped payload byte (mount refused, file service unavailable, no panic, owner control usable) and on one with a torn newest header (older generation read exactly). This proves guest read/remount, dormant staging and a control-only start; installation, broader authority grants for storage-sourced applications and the full #51 capacity/failure workload remain pending.

## Not claimed

This document selects a workload and budget, not production capacity or general service support. The numbers above are measurements of one build on one machine; the 64 MiB figure is the issue's planning target and not a product promise. The host-tested v7 owner implements bounded replacement, durable admission/execution/cancellation, explicit retention maintenance and an out-of-place v5 converter. A narrow read-only guest profile has now mounted the host-provisioned V7 image and verified the full selected ELF and manifest across reboot and service restart. Mount-time corrupt-input detection has host agreement between two readers and a guest case for a flipped payload byte and a torn newest header. Guest tracked writes up to the retained budget, revocation during a guest transfer, cold receipt lookups and owner retention maintenance through the service (repeated fill, maintain and write cycles, old-epoch retries and lookups `ExpiredEpoch`) are now verified in disposable fixtures. Staged admissions over the V7 service are verified too: a 64 KiB admission stays admitted across reboot, blocks maintenance with `Busy`, is executed with a matching completion receipt, and a second admission overtaken by a tracked write is refused with `Version` and then explicitly cancelled with the `requested` cause ([V7 staged admissions](FILES-V7-ADMISSIONS.md)). So are writes and maintenances interrupted at every publication boundary by a fail-stop device error, which report `Uncertain` and agree with the independent reader after restart and reboot, and system memory that stays at its idle value across large writes, faults and restarts, with owner `INFO` round trips of at most one 10 ms tick (under 20 ms) during a 512 KiB write ([interrupted publication](FILES-V7-WRITES.md#interrupted-publication), [memory and control latency](FILES-V7-WRITES.md#memory-and-control-latency)). Storage exhaustion at the selected 64 MiB budget and the time the single-loop service is blocked in a commit (at most 33 ticks observed, independent of size), a maintenance (at most 3 ticks) and a full-volume mount (at most 2,436 ticks) are measured in the guest ([blocking sections](FILES-V7-WRITES.md#blocking-sections-commit-maintenance-and-mount)); these are observed maxima under QEMU TCG, not bounds. Launch beyond a control-only `utility` from the workspace, guest cases for other damaged metadata and other fault shapes (torn or lost acknowledged writes, a stalled V7 device) remain open; V7 I/O has no deadline of its own; #51 is not complete.

## Stage two: format decision

Three shapes were considered for the budget selected above.

| Shape | What it keeps | Cost | Fit |
| --- | --- | --- | --- |
| **Extend the current layout in place** — wider length, one bank per file replaced by a bank chain, larger fixed sector budget | The verified copy-on-write commit, version pinning, receipts and recovery evidence | The fixed `(slot, bank)` address formula does not scale to 64 MiB; a chain makes read amplification and corruption reach grow with file length | Works for the small end, not for the selected budget |
| **Adopt an existing filesystem** (FAT-style cluster chains, or a journaling design such as the one `#12` references) | Mature on-disk algorithms and tooling familiarity | A no_std port plus the product's own semantics on top: FAT-style chains have neither atomic copy-on-write commit nor version/receipt records, and journaling gives metadata consistency, not the operation boundary this system promises; `docs/architecture/systems-roadmap.md` also defers a custom advanced filesystem without a concrete experiment | Rejected for this stage |
| **Extend the copy-on-write metadata, split the payload** — keep `Node`/version/receipt structures as the control records, widen `length` beyond 16 bits, and move file bytes into a dedicated extent region owned by a free-space map, with a typed total-data cap | Every guarantee that is already evidenced: atomic publication, version conflicts, receipts, interrupted-write recovery, and the authority model that sits above them | New work: extent allocation, free-space accounting, typed exhaustion, and a deliberate v5 to v6 upgrade | **Selected** |

The mechanism classes are standard: block chains (as FAT-style filesystems use) trade random access and corruption blast radius for simplicity, while extent lists (as in ext4's extent tree) describe runs and bound the metadata per file. This system's payload is mostly whole-artifact writes and range reads, so runs are the natural unit and the control records stay small. The comparison here is a design judgement from those references, not a measured benchmark; `#12` owns original storage acceptance and the roadmap's citation of [ext4 journaling](https://www.kernel.org/doc/html/latest/filesystems/ext4/journal.html) is used for its architectural lesson, not as an adoption argument.

**Selected shape, concretely:** control records stay in the existing copy-on-write structures with `length` widened; payload occupies a dedicated region addressed by an extent per file (one or more runs), with a free-space map that is itself checksummed; a typed cap bounds total data, per-file size and object count separately, and exhaustion is reported honestly rather than by eviction. Retention grows to eight operation records, and its maintenance path stays a separate, explicit stage.

**Why this is the robust choice for this project:** it is the only shape that keeps the guarantees already demonstrated on this volume — atomic commit, version conflict detection, receipts and recovery — while changing the part that actually limits capacity, the payload placement. Adopting a foreign on-disk format would put those guarantees on new, unreviewed ground and make the existing recovery evidence state something weaker than it does today.

## Stage three: what stage one and two decided, implemented

The layers below are in the tree with host tests; guest-side integration and the disposable-volume acceptance are still owed (see the issue for the remaining bullets).

| Layer | Owner | What it fixes |
| --- | --- | --- |
| Extent types | `crates/fs/src/extent.rs` | One file holds at most eight runs and 512 sectors; the free-space map is first-fit, refuses a double release and reports typed exhaustion |
| Control records | `crates/fs/src/format6.rs` | 128-byte nodes with `length: u32`, two generations, a header that checksums nodes, map and receipt block together |
| Publication | `crates/fs/src/volume6.rs` | Payload allocated and written before the record that points at it; one header write publishes a completely built inactive generation, so a torn commit cannot mount as truth |
| Receipts | `crates/fs/src/receipt6.rs` | Eight retained records in the published generation; a full table is `Full`, never an eviction, and the epoch cannot rotate while a record is held |
| v5 upgrade | `crates/fs/src/upgrade6.rs` | Deliberate, one-way migration that preserves identity, names, versions, kinds, spaces and bytes |
| Independent reader | `tools/terminal_support/oracle6.py`, `tools/fs6_test.py` | Reads images the Rust code wrote, from the format description and Python's own CRC, and records what it refuses |
| Host tool | `tools/volume` (package `rustic-volume`) | `provision`, `seed`, `migrate` and `report` real image files, so the v6 layer and the migration are executable outside a test |
| In-place mount | `Volume6::mount_into`, `Volume6::EMPTY` | Mounts into caller-provided storage, so a process whose stack cannot hold the 48 KiB table keeps it in a static; `mount` stays the value-returning convenience |
| Bounded reads | `Volume6::read_range` | Reads a byte range by streaming only the extents it touches, so a consumer needs no buffer for the whole file; `read_file` is the whole-file case |
| Operation identity | `Volume6::write_tracked` | One commit publishes the bytes, the bumped version and the receipt that names the operation, so a replay after an unknown outcome repeats the identity instead of the work |

**What the independent reader found.** Two defects that the in-crate tests did not: `Node6::decode` accepted a `name_length` beyond the 32 bytes it holds (a panic waiting on media this code did not write) and a record claiming more than the 512-sector per-file cap, and `Volume6::stage_bytes` never released the runs a rewritten file replaced, so every rewrite leaked its old payload until the region filled. The first two are refused by `decode` now, the third releases the replaced runs after the new payload is written, and the reader checks the free-space map against the live records so a leak cannot pass unnoticed. The 200 kB image fixture is what exposed the leak: a shorter rewrite of the same file held 197 sectors instead of 1.

**Operation identity, concretely.** `write_tracked` refuses a full table (`Full`, never an eviction), a stale epoch (`ExpiredEpoch`), a foreign lineage and a version conflict before it stages anything, then writes the payload, bumps the node version and retains the receipt in one commit, so the data and the evidence of it are published by the same header write. Replaying a retry returns the retained receipt without writing; a retry that describes another record is `IdempotencyConflict`. A v6 receipt carries no snapshot of the replaced bytes, so a replay matches identity, previous version and length rather than content — that difference from the v5 record is deliberate and is what still refuses a migration with retained v5 evidence.

**Publication failure discipline.** Payload and inactive-generation structures must pass a durability flush before the publishing header is written; a second flush confirms that header before it is adopted in memory. Once publication I/O fails, the instance reports `Uncertain` for further mutations, payload reads and receipt lookup/replay. Mounting again requires a successful flush before decoding, so a cached but unsettled header cannot be trusted as durable recovery. Allocation refusals before I/O release their reservations and leave the instance usable; a failed in-place mount leaves it fenced. This adds no on-disk format, syscall or service contract. Public structure fields remain low-level construction/inspection state, not proof of a committed operation.

The failure model distinguishes ordered durability from sector atomicity: the single checksummed header has no redundant recovery copy. A torn header may therefore produce `Corrupt`; these changes do not promise recovery from every partial-sector write or a device that violates successful flush ordering. Host fault injection and an ordinary guest write/reboot/replay are separate evidence; neither demonstrates guest-side injected failure acceptance for the production file service.

**Upgrade rules, concretely.** The v5 layout overlaps v6 (header at sector 8, banked data from sector 32, recovery state from sector 160; v6 keeps header, node tables, maps and receipts in 8..205), so an in-place upgrade reads the v5 records, stages each file's payload into the free v6 region at sector 205 and above, and only then publishes. Two states are refused instead of being dropped: a retained v5 recovery record, because the v5 record snapshots the original file bytes as evidence and the v6 receipt table carries no such snapshot yet, and a v5 provisioning envelope with a different lineage, because that is another volume's identity. A v5 volume with no retained record migrates, and its identity watermark is reported so the caller can seed allocation: v6 deliberately carries no allocator.

**Guest read and write, concretely.** The `block-user` boot mode runs a fifth `block-probe` role (14) on a host-provisioned v6 volume placed at sector 1024 of the disposable disk: the guest mounts the volume in place through the real virtio block device, checks the artifact's identity and length, reads 16 KiB in four 4 KiB ranges and folds a 64-bit FNV digest. On the first boot it also creates a file whose bytes are that digest and publishes it with `write_tracked`, so the data, the bumped version and the receipt are committed together; the second boot must find the receipt and the bytes, verify the version the receipt names, and replay the same retry without writing a sector. `tools/boot_support/workspace_evidence.py` writes that volume with `rustic-volume` (the same writer the tests use), re-reads it with `terminal_support.oracle6`, recomputes the digest from its own artifact, checks the receipt binding the reader enforces and requires the volume digest to be identical after the replay boot, so agreement is between the guest, the writer and an independent reader. The mode's documented floor is 90 s because the mount alone costs 96 real block requests and the guest's tracked write commits a whole generation; the artifact is 16 times the v5 file limit and the guest's read buffer is 4 KiB, which is what makes the bounded-RAM claim concrete.

**What the upgrade does not promise.** It is one-way and needs a caller-owned backup. A publish torn between the v6 structure writes and the header write leaves the old v5 header in place over data the structure writes have already overwritten, so a torn publish is detected by v6 refusing to mount, not by v5 still being usable. The migration also drops the v5 recovery capability flags, which describe how new evidence would be captured and retain nothing by themselves. It does clear the v5 second metadata bank before it publishes, because that header sits outside the v6 structures and a later v5 mount could otherwise republish the old layout over the new one. Nothing of this runs on a guest yet.

## Next stages of #51

The production successor is now specified separately in
[format 7 and native payload profile 2](WORKSPACE-FORMAT7.md). Its codecs persist
the identity watermark and scoped candidate extent snapshots that v6 lacks.
The separate `Volume7` owner now destructively provisions fresh/disposable media,
verifies mounts, replaces an existing file with a tracked direct commit, and
explicitly retires terminal records by advancing the retry epoch and reclaiming
unowned snapshots. Publication uses the inactive metadata generation and matching
header; this is not a migration path or production service backend. The v5/v6
formats remain frozen.

1. The host-tested v7 owner provisions and validates mounts, publishes a narrow `DirectCommitted` replacement with version conflicts, exact-byte retry and dual-generation copy-on-write, and explicitly advances retry epochs to reclaim terminal snapshots. The v6 direct probe remains a separate tested precursor, not the production backend.
2. Implement bounded staged writes, durable admission and pollable cancellation without evicting unresolved evidence. Add deliberate compatibility/upgrade behavior on disposable copies. The owner now also accepts a file one sector at a time through host-tested streamed stages with in-memory reservations ([streamed staging](WORKSPACE-FORMAT7.md#streamed-staging)); stage writes are not pollable yet and no service uses them.
3. Integrate the service and explicitly selected native profile, then run the selected consumer above today's limits. Independently verify data, versions and operation identity under full storage/retention, interrupted publication, reboot/remount, corrupt input and bounded RAM/control latency.

Deliberate data migration is evidenced separately from executable rollback:
`rustic-volume migrate7` converts disposable v5 images built by
`seed5-history`, and `tools/v7_migration_test.py` exercises the migrated direct
commit, admission, cancellation and executed admission in the guest
([deliberate data migration](WORKSPACE-FORMAT7.md#deliberate-data-migration-of-a-disposable-image)).
Records from real v5 terminal volumes carry subject 1 and stay invisible to the
V7 shell (subject 2); subject remapping and the 4 GiB terminal disk size are
outside this step.

**Data migration and executable rollback are separate.** Data migration is a
one-way host step that converts a v5 image into a new V7 image and never
mutates its source (its SHA-256 is checked before, after and, in the rollback
run, after the guest boot). Executable rollback is a guest selection on
unchanged data: `rustic-volume add7` publishes a tag-1 and then a tag-2
`utility` pair beside a migrated history, and one boot of the unchanged kernel
image starts the tag-2 pair and then the older tag-1 pair with the volume, its
`oracle7` view and its migrated records unchanged
([executable rollback](NATIVE-RUNTIME.md#executable-rollback-on-one-migrated-volume),
[adding a file](WORKSPACE-FORMAT7.md#adding-a-file-to-a-disposable-image)).
Rollback here is owner-pinned selection only: there is no persistent "current
version" pointer or activation record, "older" means published earlier (both
variants declare the same manifest version), and SHA-256 pins do not
authenticate a publisher. Nothing rolls data back.
