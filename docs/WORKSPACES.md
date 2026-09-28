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

- **512 KiB per V7 file** — covers the current largest shipped executable with about 61% headroom. V5 limits remain frozen. A 512 KiB feature bit distinguishes this V7 profile from pre-release 256 KiB V7 images; old-format development fixtures must be reprovisioned.
- **256 objects** — eight times today's 32, enough for application generations plus configuration and task records.
- **64 MiB of file data per volume** — the issue's planning target, independent of the 256-object and 512 KiB per-file ceilings. Not all maximum-size files can coexist; total capacity is exhausted first. `python3 tools/v7_capacity_test.py` serves a volume with 200 objects and 130,972 of the 131,072 payload sectors allocated in the guest: objects 48, 72 and 200 read by reference, writes the 100 free sectors cannot hold are refused with `Full` with no transfer left open and the image unchanged, a smaller write then commits, and retention maintenance frees exactly the snapshot-only sectors so a refused write fits afterwards. The 256-entry object table is filled on the host with `add7`, which then refuses; the V7 service has no create request, so the guest cannot reach that limit ([storage exhaustion](FILES-V7-WRITES.md#storage-exhaustion-on-a-nearly-full-volume)). Mounting such a volume verifies every allocated payload sector and took 2,426 and 2,436 guest ticks in two runs (about 24 s under QEMU TCG); the supervisor's V7 start and restart jobs now have a measured 6,000-tick budget, because the ordinary 1,000-tick deadline timed out above roughly 22 MiB of allocated payload.
- **8 retained operation records** — four times today's two, so a client can recover a small window of unresolved outcomes instead of losing the third mutation to `Full`. Retention is bounded; v7 now has an explicit retry-epoch transition that refuses open admissions, and unresolved evidence is never silently dropped. In `mode=terminal-v7` the owner runs it as the `MAINTAIN_V7` job, refused with `Busy` while a transfer is open, and the guest harness shows three fill, maintain and write cycles across a reboot ([owner retention maintenance](FILES-V7-WRITES.md#owner-retention-maintenance)).

**Format follow-up:** at the time this budget was selected, the next design was described as a v6 layout with extents per file, a wider length field, typed total-data exhaustion and deliberate upgrade behavior from v5; that v6 prototype and its upgrade were later removed. The later [format-7 decision](WORKSPACE-FORMAT7.md) supersedes that target to persist scoped identity, immutable retry snapshots, staged admission and explicit retention without weakening the selected budget. No converter or migration into V7 exists; V7 images are provisioned fresh ([storage policy](STORAGE-POLICY.md)). An explicitly selected V7 service is wired behind `mode=terminal-v7`; v5 remains the default. The V7 service now also accepts streamed profile-2 tracked writes up to 512 KiB from the shell, and `python3 tools/v7_write_test.py` fills the eight-record budget, refuses the next write with `Full` and replays an exact retry after reboot with an identical receipt and no volume change ([V7 tracked writes](FILES-V7-WRITES.md)). `python3 tools/v7_read_test.py` now provisions a fresh temporary V7 volume and reads the complete `file-server.elf` plus manifest in two QEMU boots, including a service restart, while checking byte equality and an unchanged volume digest. The same harness now has the supervisor stage that pinned ELF/manifest pair as a dormant, never-started child and refuse stale pins ([storage-sourced dormant images](NATIVE-RUNTIME.md#storage-sourced-dormant-images-modeterminal-v7)). `python3 tools/v7_launch_test.py` then boots one frozen kernel image against two fresh V7 volumes holding separately built `utility` variants and starts each staged child with a control-only topology ([starting the staged child](NATIVE-RUNTIME.md#starting-the-staged-child-control-only)). `tools/terminal_support/oracle7.py` is an independent V7 reader written from the format document; `python3 tools/fs7_test.py` checks it against `report7` on host-written images with every retained-record state and records, per damaged copy, that it and the Rust mount agree on what is refused (see [independent reader and damaged input](WORKSPACE-FORMAT7.md#independent-reader-and-damaged-input)). `python3 tools/v7_corrupt_test.py` boots the V7 fixture on a volume with a flipped payload byte (mount refused, file service unavailable, no panic, owner control usable) and on one with a torn newest header (older generation read exactly). This proves guest read/remount, dormant staging and a control-only start; installation, broader authority grants for storage-sourced applications and the full #51 capacity/failure workload remain pending.

## Not claimed

This document selects a workload and budget, not production capacity or general service support. The numbers above are measurements of one build on one machine; the 64 MiB figure is the issue's planning target and not a product promise. The host-tested v7 owner implements bounded replacement, durable admission/execution/cancellation and explicit retention maintenance. A narrow read-only guest profile has now mounted the host-provisioned V7 image and verified the full selected ELF and manifest across reboot and service restart. Mount-time corrupt-input detection has host agreement between two readers and a guest case for a flipped payload byte and a torn newest header. Guest tracked writes up to the retained budget, revocation during a guest transfer, cold receipt lookups and owner retention maintenance through the service (repeated fill, maintain and write cycles, old-epoch retries and lookups `ExpiredEpoch`) are now verified in disposable fixtures. Staged admissions over the V7 service are verified too: a 64 KiB admission stays admitted across reboot, blocks maintenance with `Busy`, is executed with a matching completion receipt, and a second admission overtaken by a tracked write is refused with `Version` and then explicitly cancelled with the `requested` cause ([V7 staged admissions](FILES-V7-ADMISSIONS.md)). So are writes and maintenances interrupted at every publication boundary by a fail-stop device error, which report `Uncertain` and agree with the independent reader after restart and reboot, and system memory that stays at its idle value across large writes, faults and restarts, with owner `INFO` round trips of at most one 10 ms tick (under 20 ms) during a 512 KiB write ([interrupted publication](FILES-V7-WRITES.md#interrupted-publication), [memory and control latency](FILES-V7-WRITES.md#memory-and-control-latency)). Storage exhaustion at the selected 64 MiB budget and the time the single-loop service is blocked in a commit (at most 33 ticks observed, independent of size), a maintenance (at most 3 ticks) and a full-volume mount (at most 2,436 ticks) are measured in the guest ([blocking sections](FILES-V7-WRITES.md#blocking-sections-commit-maintenance-and-mount)); these are observed maxima under QEMU TCG, not bounds. Launch beyond a control-only `utility` from the workspace, guest cases for other damaged metadata and other fault shapes (torn or lost acknowledged writes, a stalled V7 device) remain open; V7 I/O has no deadline of its own; #51 is not complete.

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

## Stage three: the v6 prototype (removed)

The stage-two shape was first implemented as a v6 layout: extent types, copy-on-write
control records with a wider length, eight retained receipts, an in-place v5 upgrade,
an independent Python reader, a host tool and a `block-user` guest role that mounted,
read and wrote a host-provisioned v6 volume. Its receipts carried no snapshot of the
replaced bytes and it persisted no identity watermark, so it could not carry the v5
service's durable admission and retry contracts; [format 7](WORKSPACE-FORMAT7.md)
replaced it. The v6 layout, its upgrade, reader, host commands and guest role were
removed and no data was carried from them ([storage policy](STORAGE-POLICY.md)). The
extent types in `crates/fs/src/extent.rs` remain because V7 uses them.

## Next stages of #51

The production successor is now specified separately in
[format 7 and native payload profile 2](WORKSPACE-FORMAT7.md). Its codecs persist
the identity watermark and scoped candidate extent snapshots that the removed v6 prototype lacked.
The separate `Volume7` owner now destructively provisions fresh/disposable media,
verifies mounts, replaces an existing file with a tracked direct commit, and
explicitly retires terminal records by advancing the retry epoch and reclaiming
unowned snapshots. Publication uses the inactive metadata generation and matching
header; this is not a production service backend. The v5 format remains frozen.

1. The host-tested v7 owner provisions and validates mounts, publishes a narrow `DirectCommitted` replacement with version conflicts, exact-byte retry and dual-generation copy-on-write, and explicitly advances retry epochs to reclaim terminal snapshots.
2. Implement bounded staged writes, durable admission and pollable cancellation without evicting unresolved evidence. No compatibility or upgrade path is added while no user volume exists ([storage policy](STORAGE-POLICY.md)). The owner now also accepts a file one sector at a time through host-tested streamed stages with in-memory reservations ([streamed staging](WORKSPACE-FORMAT7.md#streamed-staging)); stage writes are not pollable yet and no service uses them.
3. Integrate the service and explicitly selected native profile, then run the selected consumer above today's limits. Independently verify data, versions and operation identity under full storage/retention, interrupted publication, reboot/remount, corrupt input and bounded RAM/control latency.
