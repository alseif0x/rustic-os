<!-- SPDX-License-Identifier: Apache-2.0 -->

# Application workspaces: measured limits and selected budget (#51)

Stage one of [#51](https://github.com/alseif0x/rustic-os/issues/51): measure the consuming workload, state the structural limits that a larger workspace has to change, and select an explicit acceptance budget. No capacity is implemented by this document.

## Measured consumers

The native application artifacts that a workspace must eventually carry, as built on this machine:

| Artifact | Bytes |
| --- | ---: |
| `file-server.elf` | 215,296 |
| `shell.elf` | 208,824 |
| `block-probe.elf` | 186,024 |
| `utility.elf` | 126,560 |
| `supervisor.elf` | 68,128 |
| `tasks.elf` | 52,808 |
| `sdk-probe.elf` | 25,944 |
| every `*.manifest` | 128 |

The largest shipped executable is **215,296 bytes**, and the manifest that identifies it is 128 bytes. Anything that installs or launches an application from a workspace (#52) therefore needs files of that order, not of the current order.

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
2. **The length field is 16 bits.** Even with extent mapping, `Node.length: u16` caps any file at 65,535 bytes — 3.3 times below the largest artifact measured above.
3. **The fixed structures are small.** 174 sectors bound the whole volume's payload area; a 64 MiB workspace is not a constants change but a different allocation scheme.

Whole-file buffers are the other bound: the file service holds `Transfer.data: [u8; MAX_FILE]` for two slots and the filesystem layer keeps whole-file buffers, so raising the per-file limit multiplies RAM per slot by the same factor. A 1 MiB file with two slots is 2 MiB of RAM before any copy, which must be measured against the reference profile rather than assumed.

## Selected workload and budget

**Workload:** install and launch one native application artifact from a workspace, the consumer that #52 names. Its measured size is 215,296 bytes plus a 128-byte manifest.

**Budget, pinned to that consumer:**

- **256 KiB per file** — covers the largest shipped executable with about 19% headroom; 1 MiB stays a planning ceiling to be revisited only with a larger measured artifact.
- **256 objects** — eight times today's 32, enough for application generations plus configuration and task records.
- **64 MiB of file data per volume** — the issue's planning target, kept as the total cap because 256 x 256 KiB would otherwise be 64 MiB exactly; the cap is what makes exhaustion decidable.
- **8 retained operation records** — four times today's two, so a client can recover a small window of unresolved outcomes instead of losing the third mutation to `Full`. Retention is bounded and its eviction policy belongs to the next stage; unresolved evidence is never silently dropped.

**What this implies for the format:** a v6 layout with extents (or a bank chain) per file, `Node.length` widened beyond 16 bits, an explicit total-data cap with typed exhaustion, and deliberate upgrade behavior from v5. Those are the implementation stages of #51, not this document.

## Not claimed

No capacity is implemented, no format is selected, and no filesystem is preselected by this document. The numbers above are measurements of one build on one machine; the 64 MiB figure is the issue's planning target and not a product promise. Whole-file RAM cost, interruption behavior, retention maintenance and migration are named here and remain to be measured and implemented in the following stages.

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
| Bounded reads | `Volume6::read_range` | Reads a byte range by streaming only the extents it touches, so a consumer needs no buffer for the whole file; `read_file` is the whole-file case |
| Operation identity | `Volume6::write_tracked` | One commit publishes the bytes, the bumped version and the receipt that names the operation, so a replay after an unknown outcome repeats the identity instead of the work |

**What the independent reader found.** Two defects that the in-crate tests did not: `Node6::decode` accepted a `name_length` beyond the 32 bytes it holds (a panic waiting on media this code did not write) and a record claiming more than the 512-sector per-file cap, and `Volume6::stage_bytes` never released the runs a rewritten file replaced, so every rewrite leaked its old payload until the region filled. The first two are refused by `decode` now, the third releases the replaced runs after the new payload is written, and the reader checks the free-space map against the live records so a leak cannot pass unnoticed. The 200 kB image fixture is what exposed the leak: a shorter rewrite of the same file held 197 sectors instead of 1.

**Operation identity, concretely.** `write_tracked` refuses a full table (`Full`, never an eviction), a stale epoch (`ExpiredEpoch`), a foreign lineage and a version conflict before it stages anything, then writes the payload, bumps the node version and retains the receipt in one commit, so the data and the evidence of it are published by the same header write. Replaying a retry returns the retained receipt without writing; a retry that describes another record is `IdempotencyConflict`. A v6 receipt carries no snapshot of the replaced bytes, so a replay matches identity, previous version and length rather than content — that difference from the v5 record is deliberate and is what still refuses a migration with retained v5 evidence.

**Upgrade rules, concretely.** The v5 layout overlaps v6 (header at sector 8, banked data from sector 32, recovery state from sector 160; v6 keeps header, node tables, maps and receipts in 8..205), so an in-place upgrade reads the v5 records, stages each file's payload into the free v6 region at sector 205 and above, and only then publishes. Two states are refused instead of being dropped: a retained v5 recovery record, because the v5 record snapshots the original file bytes as evidence and the v6 receipt table carries no such snapshot yet, and a v5 provisioning envelope with a different lineage, because that is another volume's identity. A v5 volume with no retained record migrates, and its identity watermark is reported so the caller can seed allocation: v6 deliberately carries no allocator.

**What the upgrade does not promise.** It is one-way and needs a caller-owned backup. A publish torn between the v6 structure writes and the header write leaves the old v5 header in place over data the structure writes have already overwritten, so a torn publish is detected by v6 refusing to mount, not by v5 still being usable. The migration also drops the v5 recovery capability flags, which describe how new evidence would be captured and retain nothing by themselves. It does clear the v5 second metadata bank before it publishes, because that header sits outside the v6 structures and a later v5 mount could otherwise republish the old layout over the new one. Nothing of this runs on a guest yet.

## Next stages of #51

1. Implement the v6 layout with typed limits, honest exhaustion and upgrade behavior; never silently rotate an epoch or evict unresolved outcomes. Implemented in the tree, host-tested (stage three above); the file service does not mount it yet.
2. Extend the admission and reclamation rules to the new capacity, preserving unresolved evidence. Not started; v6 currently refuses to migrate a volume that still retains recovery evidence.
3. Run the selected consumer on a disposable volume above today's limits and verify data, versions and operation identity independently, with interrupted publication, reboot/remount, corrupt input and bounded RAM. Not started; it needs stage one of the two above.
