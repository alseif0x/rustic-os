<!-- SPDX-License-Identifier: Apache-2.0 -->

# Workspace format 7 and native payload profile 2

Decision for [#51](https://github.com/alseif0x/rustic-os/issues/51), adopted
2026-09-22 under the owner's authorization to continue implementation and
versioned storage/protocol decisions. On 2026-09-23 the native artifact
measurements were refreshed: `file-server.elf` was 324,344 bytes and
`block-probe.elf` 283,816 bytes. V7/profile 2 therefore now supports 512 KiB
per file. On 2026-09-24, after the V7 tracked-write service and its receipt
lookups, `file-server.elf` measured 364,104 bytes (it grew with the write
service) and `block-probe.elf` 283,544 bytes; both still fit 512 KiB; V5/V6 remain frozen. Feature bit 3 identifies this development profile
and the reader refuses earlier 256 KiB V7 images, which must be reprovisioned.
The current host-tested owner supports
tracked replacement, durable staged admission, explicit admitted execution,
cause-bearing cancellation and explicit retry-epoch retention maintenance.
Payloads and exact retries use bounded sector I/O; candidate bytes and inactive
metadata are flushed before the alternate header, and owner state changes only
after final flush settlement. Maintenance refuses open admissions and must only
run after the service has resolved current-epoch outcomes. The host-only
`upgrade_v5_to_v7` converter copies a v5 source to distinct disposable media,
preserving supported scoped recovery records and refusing ambiguous or invalid
history. The explicit `mode=terminal-v7` fixture now selects a V7 service with
bounded reads and [profile-2 tracked writes](FILES-V7-WRITES.md) announced in
its readiness report; production/default `mode=terminal` remains v5-backed, and
no capability advertisement is wired. The v6 direct probe remains separate.

## Why a successor

The [measured workspace budget](WORKSPACES.md) selects 256 live objects, 64 MiB
of payload, 512 KiB per V7 file and eight retained outcomes. The v6 extent prototype
supports that payload geometry but does not persist the monotonic identity
watermark or the subject/workspace/instance binding and exact candidate bytes
required by the v5 service's durable admission and retry contracts. Substituting
it directly would weaken authority and replay semantics.

Keep v5 and v6 frozen. Extend their copy-on-write design in an explicitly new
layout: bounded control records reference immutable candidate extent snapshots.
This retains exact-byte retry comparison without embedding whole large files in
each record or replacing equality with a digest. A different filesystem would
still require these service-specific records and recovery rules; a digest-only
extension would not preserve the existing contract. No dependency, heap, kernel
ABI change or new unsafe boundary is introduced.

## Disk layout

All numbers are little-endian. Sectors are 512 bytes relative to the volume.
The magic is `RUSTFS3\0`, version 7, layout 1, exact feature mask 15 (extent
payload, immutable snapshots, scoped records, 512 KiB files). Unknown features
and pre-release mask-7 256 KiB images are refused.

| Region | Generation 0 | Generation 1 | Size |
| --- | --- | --- | --- |
| Header | sector 8 | sector 9 | 1 sector each |
| Nodes | sector 10 | sector 110 | 256 records of 128 bytes |
| Allocation map | sector 74 | sector 174 | 16,384 bytes |
| Retained records | sector 106 | sector 206 | 8 records of 192 bytes, then 512 zero bytes |
| Shared payload | sector 210 | same region | 131,072 sectors |

Total volume size is 131,282 sectors. Each node or retained record references at
most eight runs, each two `u32` values (start and sector count), relative to the
payload. Runs must be nonempty, in bounds, nonoverlapping within the record, and
total exactly `ceil(length / 512)`; unused runs are zero. Length is `u32`, capped
at 524,288 bytes. An empty file has no runs.

The header copy for generation `g` is stored in sector `8 + g`:

| Byte | Field |
| --- | --- |
| 0 / 8 / 9 / 10 / 11 | magic, version `u8` 7, layout `u8` 1, generation `u8`, reserved zero |
| 12 / 28 / 36 / 44 | lineage (16 bytes), retry epoch `u64`, global sequence `u64`, next identity `u32` |
| 48 / 52 / 56 | node table, allocation map and retained-record block aggregate CRCs, `u32` |
| 60 / 64 / 68 / 72 | objects 256, node bytes 128, map bytes 16,384, payload sectors 131,072, all `u32` |
| 76 / 80 / 84 / 88 | retained records 8, feature mask 15, record bytes 192 (`u32`); runs per record 8 (`u8`) |
| 89..508 / 508 | reserved zero / header CRC |

The CRC at 508 covers the entire sector with that field zeroed. Epoch and sequence
start at 1; epoch cannot exceed sequence. Next identity starts at 5 and never
means live-object count. `u32::MAX` is the exhausted watermark, not an allocatable
identity. Generation must be 0 or 1. The lineage is nonzero.

Every CRC in this format is CRC-32/IEEE (reflected polynomial `0xEDB88320`,
initial value and final XOR `0xFFFFFFFF`, check value `0xCBF43926` for
`123456789`; the same function as zlib's `crc32`). Each aggregate covers the raw
bytes of its whole region in the named generation: all 32,768 node-table bytes,
all 16,384 map bytes, and all 2,048 retained-record block bytes including the
512 zero padding bytes. The allocation map is 2,048 little-endian `u64` words;
payload sector `s` is bit `s % 64` of word `s / 64`, and a set bit means
allocated.

Nodes store identity `u32`/parent `u32`/version `u64`/length `u32` at 0/4/8/16,
kind/space/run count/name length (`u8` each) at 20/21/22/23, runs at 24, a
zero-padded 32-byte name at 88, payload CRC at 120 and a record CRC over bytes
0..124 at 124. Kind 1 is a file and 2 a directory; space is 1..=4; identity and
version are nonzero. Directory payload is empty (length, payload CRC and run
count zero). An empty node slot is all zero. Names keep the existing namespace
rules: 1..=31 bytes of ASCII letters, digits, `.`, `_` or `-`, excluding `.`
and `..`, zero padded to 32 bytes. Per-node decoding does not establish
parentage, namespace validity or global allocation ownership.

A payload CRC (node byte 120, record byte 76) covers exactly `length` bytes read
in run order; an empty payload's CRC is 0. The bytes after `length` in the final
sector are not covered: writers zero them, and readers neither check nor return
them.

Retained record offsets:

| Byte | Field |
| --- | --- |
| 0 / 8 / 12 / 16 | subject `u64`, workspace `u32`, object `u32`, service instance `u64` |
| 24 / 32 | retry epoch / key, both `u64` |
| 40 / 48 / 56 / 64 | previous / committed / admission / terminal sequence, all `u64` |
| 72 / 76 | candidate length / payload CRC, both `u32` |
| 80 / 81 / 82 | state / prevention cause / run count, each `u8` |
| 88 | eight extent pairs |
| 188 | CRC over bytes 0..188 |

State bytes are 0 direct committed, 1 admitted, 2 cancelled and 3 admitted
committed. A cancelled record's cause byte is 0 unknown (legacy), 1 requested,
2 version conflict or 3 authority lost; the cause byte is 0 in every other
state. Other bytes (83..88 and 152..188) are reserved zero. Lineage is shared through the header. Subject,
workspace, instance, previous version and retry fields are nonzero; object is above the four root
identities and differs from workspace. Contextual validation requires both
identities below the watermark and the retry epoch, previous version and all
sequences at most the header sequence.

States preserve the existing prevention causes, including unknown legacy cause:

- Direct committed: no admission; terminal equals committed, greater than previous.
- Admitted: admission greater than previous; committed and terminal are zero.
- Cancelled: terminal greater than admission, admission greater than previous;
  committed is zero and a prevention cause is present.
- Admitted committed: terminal equals committed, greater than admission, which
  is greater than previous.

Instance and retry epoch cannot postdate creation: committed sequence for a
direct record, admission sequence otherwise. Causes are absent outside cancelled
records. Empty receipt slots are all zero, distinct from a decoded live record.
CRCs detect corruption; they do not authenticate storage or prove payload equality.

## Whole-generation validation

`format7::validate_generation` checks decoded structures together before a future
mount owner trusts them. It requires the four named roots, unique live identities
and sibling names, same-space directory ancestry without cycles, live IDs below
the identity watermark and live versions at most the header sequence. The roots
are directories with parent 0: identity 1 `system`, 2 `data`, 3 `config` and 4
`workspaces`, each in the space equal to its identity; no other node has parent
0, and every other node's parent is a live directory in the same space. Retained records must belong to the current
retry epoch, have unique retry identities and event sequences, and agree with any
still-live target's kind and version history. Successful records for the same
object must form a monotonic commit history, and all retained version
observations must be nondecreasing by event sequence. An admitted commit also
requires every retained observation between its admission and commit to see the
same previous version. An unresolved admission may become stale after a later
commit, but a still-live target cannot have advanced past its recorded previous
version before that admission's sequence.

Precisely, a record observes its previous version at its committed sequence
when direct and at its admission sequence otherwise. For two records on one
object: their observation sequences differ and the later observation's previous
version is not lower; an admitted commit requires any other observation strictly
between its admission and commit to have the same previous version; an
observation after any commit has a previous version at least that commit; and of
two commits, the later one's previous version is at least the earlier commit.
Across all records, retry identities (subject, workspace, epoch, key) are unique
and no nonzero admission, committed or terminal sequence is shared. A still-live
target must be a file whose version is at least the record's previous version;
for an admitted or cancelled record it is not in `(previous, admission]`; for a
committed record it is at least the commit, and when equal the node's length,
payload CRC and runs are exactly the record's. An empty retained snapshot has
payload CRC 0.

The persisted allocation map must exactly equal ownership by live file extents
and retained payload snapshots. The only shared ownership allowed is one exact
committed snapshot that aliases its current live file version. The caller supplies
the bitmap scratch space, which is cleared before validation and may contain
partial marks on failure. This function receives decoded values: it does not
verify serialized aggregate checksums, read disk, or check payload bytes against
their CRCs. It neither mounts nor publishes a generation.

## Mount and header selection

The physical slot (sector 8 or 9) must match the generation in its header. Mount
flushes the device before reading. A CRC-invalid or otherwise invalid header copy
is not a candidate; if the other copy is valid, mount may select it and must
report that it recovered from an invalid copy. The one exception to that report
is a freshly provisioned volume: a lone valid generation-0 copy with sequence 1,
epoch 1 and next identity 5 whose other copy is entirely zero is not a recovery. When both copies are valid, they
must share lineage, name opposite generations, have adjacent sequences, and have
nondecreasing epoch and identity watermark; mount selects the higher sequence.
Stale inactive metadata is not decoded while selecting the head because a
publication may already have overwritten that generation before replacing its
old header.

Once the highest valid header is selected, any named-region checksum failure,
structural inconsistency, or payload CRC mismatch is corruption. Mount must not
fall back to a lower valid header in that case: doing so could silently discard a
committed generation. CRCs detect damage; they do not establish atomic sector
writes. Recovery relies on the device honoring successful flush ordering, as
required by `Disk`.

Mount reads only the header copies, the selected generation and the payload
sectors referenced by live files and retained snapshots. It does not check the
medium's capacity: a medium that lacks a referenced sector fails the mount with
`Io`, while a missing unused sector is left to the caller. The host
`rustic-volume report7` requires an image of exactly 131,282 sectors.

## Native payload profile

`rustic_abi::files::workspace` provides profile 2 codecs separately from the
legacy `operation` module. The kernel's 64-byte packet stays unchanged.

- Replacement uses the existing identity fields, a `u32` size in `arg`, data
  count 40 and profile marker 2 in data bytes 36..40.
- Retry lookup appends the marker at data bytes 24..28. ID and part lookups append
  it at 16..20. Part offsets remain 0, 40 and 80.
- Completed receipts remain 104 bytes: size is `u32` at 64..68, marker 2 at
  68..72, SHA-256 at 72..104. Identity and version fields keep their old positions.
- Both old and new decoders reject the other's messages, even for zero-length
  or small files. Receipt fragment framing remains unchanged; complete receipt
  decoding establishes the selected profile.

This marker does not negotiate availability. The explicit V7 fixture's readiness
records the 512 KiB file limit, the eight retained-record geometry and, in word
5 bit 0, that profile-2 tracked writes are served. The service accepts stable
references, bounded reads, and profile-2 tracked replacements with their
receipts to write-authorized grants ([V7 tracked writes](FILES-V7-WRITES.md)),
and profile-2 ID and retry lookups of retained records to grants with the
inspect right. Descriptors and generic service-v1 capabilities are not
exposed. Existing clients and schema hashes retain their original
meaning and 1 KiB limit.

## Direct tracked replacement

`Volume7::provision_into` destructively formats a fresh or disposable volume; it
is not an upgrade path and does not preserve existing metadata. Together with
`Volume7::mount_into`, it establishes only the initial disk-backed owner. It
provisions canonical generation 0 or flushes, selects and verifies the highest
valid header's regions, whole-generation structure and all live/retained payload
CRCs. A torn invalid header copy can recover the other header and reports that
condition; corruption named by a valid newest header is refused instead of
silently rolling back.

`replace_tracked` accepts only a live file and a current expected version. It
preflights identity, epoch, receipt capacity, sequence and extent availability
before writing. A retry with the same scoped key streams the retained immutable
snapshot, verifies its CRC and compares exact bytes; a different request conflicts,
and retry performs no writes or flushes. A new write stages into extents free in the
selected map, validates the candidate generation, then flushes payload plus
inactive metadata, writes the matching header and flushes it. Only after that
barrier does the owner expose the new version and receipt. Old extents remain owned
while a retained snapshot references them, and the receipt table is never silently
evicted. A post-write uncertainty fences the owner until remount. If a final flush
reports an error after making the header durable, remount selects the complete new
generation; if it did not become durable, the prior head remains selected.

Payload and retry I/O use one 512-byte sector buffer. The extent planner considers
the largest free runs first, so early small holes do not hide a later contiguous
fit; files still require at most eight runs and can honestly receive `Full` when
available space cannot satisfy that geometry. `maintain_retention` is an explicit
owner operation: it refuses while any admission is unresolved, clears terminal
records, reconstructs the allocation map from live files, and publishes the next
retry epoch through the same copy-on-write barriers. Snapshot extents are freed
only when no live file owns them. Only a caller that has confirmed clients have
resolved every current-epoch result may invoke it; once durable, old-epoch retries
return `ExpiredEpoch`. It never runs implicitly to bypass a full receipt table.

`prepare_admission` stages a candidate extent snapshot as a durable `Admitted`
record without changing the live file. It preflights version, identity, receipt
capacity, sequence and free extents, then writes payload sectors and flushes them
before publishing inactive nodes/map/records and the alternate header. Exact retry
verification polls one sector at a time and compares bytes as well as CRC; its
single-sector scratch stays bounded. `prepare_execute` rechecks the live version
and explicitly commits the admitted bytes, while `prepare_cancellation` records a
prevention cause and keeps the candidate owned without changing live contents.
Cancellation drains an outstanding pre-header command; after header submission it
is too late and the caller must settle the result. Dropping or failing with
unresolved I/O fences the owner until remount. These APIs are storage primitives,
not service authorization or scheduling policy.

There is no automatic or in-place upgrade. `upgrade_v5_to_v7` reads the v5 source
without mutation and writes a distinct target whose v7 header sectors must be
zero. It preserves lineage, sequence, identity watermark, live files and scoped
retained evidence that passes v7 validation; it refuses scope-less evidence or
history that cannot be represented before writing the target. The target may be
partial after I/O failure, so it must be disposable and backed up; this is not
rollback atomicity or production service integration. Never experiment on the
owner's `artifacts/terminal/data.raw`.

## Streamed staging

The owner can also receive a file one 512-byte sector at a time, so a caller
never lends the whole candidate. `open_stage(identity, expected_version, length,
kind)` applies the same preflight as `replace_tracked` (`Stage7Kind::Tracked`) or
`prepare_admission` (`Stage7Kind::Admission`) without I/O and returns a `Stage7`
token. The token is neither `Clone` nor `Copy` and borrows nothing, so the owner
stays usable between sectors. It carries a distinct owner identity, taken from a
monotonic program-wide counter when a `Volume7` value opens its first stage,
plus a per-owner nonce, so a token from another owner value is refused with
`Invalid`. At most two stages are open at once; a third, or a second stage for
the same retry scope, is `Busy`.

A fresh stage plans its runs around the selected map and every other open
stage and reserves one free receipt slot. The reservation is an in-memory
overlay only: it is never written into the allocation map, which keeps
describing exact durable ownership, so `free_sectors` does not change while a
stage is open. `replace_tracked` and `prepare_admission` plan around the overlay
and count reserved slots, and they return `Busy` for a scope a stage holds.
`maintain_retention` returns `Busy` while any stage is open. `abort_stage`,
`release_stages` (every open stage, including those whose token was dropped;
`open_stages` counts them), a remount and any fence release reservations without
I/O and without fencing; sectors a stage already wrote stay free because no
metadata names them. The nonce survives the fence or remount, so an earlier
token is refused with `Invalid` rather than naming a later stage.

`stage_write` takes exactly `min(512, remaining)` bytes, zero-pads the final
sector and keeps a running CRC over the logical bytes. A fresh stage issues
exactly one payload write per call; a failed write is `Uncertain` and fences the
owner. A stage that matches a retained retry record reserves nothing: each call
reads one snapshot sector, updates the CRC over the stored bytes and records any
difference without stopping; a read error releases the stage with the disk error
and does not fence. `finish_tracked(&mut Stage7)` and `finish_admission` refuse a
stage of the other kind or one with sectors still expected with `Invalid` and
leave it open; any other outcome ends the stage. A retry returns the
retained record with no writes or flushes, reporting `Corrupt` for a snapshot CRC
mismatch ahead of `IdempotencyConflict` for different bytes. A fresh stage
rechecks the version, epoch, scope and receipt slot and validates the candidate
before any metadata write. A refusal such as `Version` or `NotFound` after a
concurrent commit or removal releases the stage without fencing. Otherwise it
publishes through the same barriers as `replace_tracked`, which is now itself
implemented as open/write/finish on a stage that uses no slot, so the two paths
issue identical disk command sequences. `finish_admission` returns a
`PollPublication7` that starts at the payload flush and then follows the
borrowed admission's barriers and cut points. As in the poll retry path, a
corrupt admission retry fences. Stage writes use the blocking `Disk` interface,
while the admission publication polls.

Limits: stage writes are not yet pollable, and a dropped token keeps its
reservation until `release_stages`, a fence or a remount. The V7 file service
uses tracked stages for [profile-2 tracked writes](FILES-V7-WRITES.md); no
service uses admission stages. Owner identity is
distinct only within one program; tokens are not meant to cross processes.

## Bounded range reads

`Volume7::read_range` resolves a live object ID, optionally pins its current
version, validates the node and full extent geometry before payload I/O, and
copies only the sectors intersecting the requested logical range. Its scratch
space is one 512-byte sector regardless of file size. A stale version returns
`Version`, a directory returns `IsDirectory`, an offset beyond EOF returns
`Size`, and an empty or EOF range returns zero without payload reads. An I/O
error may leave the already copied prefix in the caller's output; callers must
discard that buffer unless the method succeeds.

`Volume7::read_retained_range(disk, record, offset, out)` reads the immutable
snapshot of a retained record through the same sector walk and one-sector
scratch space. The record must equal one the owner currently retains, field for
field; any other value, including a record cleared by maintenance, is
`NotFound` without payload I/O. The snapshot need not be the live content, and
the file may have been removed. It trusts the mount-time CRC verification of
every retained snapshot, refuses an offset beyond the record's length with
`Size` and returns zero at EOF without I/O. The V7 file service streams it to
compute receipt SHA-256 values for cold lookups; host tests in
`crates/fs/tests/volume7/read.rs` cover a superseded snapshot after remount,
a removed file, forged records, bounds and an empty snapshot.

The `rustic-volume seed7` host command creates a fresh disposable V7 image
exclusively, provisions the `workspaces/application` directory, and writes the
measured ELF and manifest as tracked files. `report7` remounts the image
read-only and verifies the provisioned lineage, identities, kinds, sizes and
bytes. The boot harness consumes those canonical workspace/resource IDs; the
guest does not invent a lineage or run a provisioner.

The full live payload CRC is checked when the volume is mounted, not rescanned
for every range. Reads therefore assume that no writer changes the medium outside
the mounted `Volume7` owner. An external media change while mounted is not
detected by this API; remount validates the whole live payload CRC again. The
explicit read-only service routes the existing SDK range API to V7; the
`terminal-v7` QEMU harness verifies the selected ELF (324,344 bytes in that
recorded run; 364,104 bytes in the 2026-09-24 build) and 128-byte
manifest byte-for-byte in two boots, with another bounded read after service
restart. It does not execute the file from the workspace or exercise V7 writes.

## Independent reader and damaged input

`tools/terminal_support/oracle7.py` is a second reader written from this
document with Python's `zlib.crc32` and `hashlib`; it does not import or call
`rustic-fs`. It applies the header selection rule above, verifies the three
aggregates, every node and record CRC, the namespace and root rules, the
retained-record state, scope, epoch and same-object history rules, exact map
ownership (live files plus retained snapshots, the one exact committed alias
excepted) and every live and retained payload CRC. It lists each file's path,
identity, version, size and SHA-256 and each retained record's state, cause,
sequences, snapshot SHA-256 and target binding. Like `report7`, it reads only an image of
exactly 131,282 sectors. Writing it exposed only documentation gaps (CRC function,
aggregate coverage, header geometry offsets, map bit order, kind/state/cause
byte values, name and root rules, payload padding, the genesis recovery
exception, version bound, the precise temporal rules and mount capacity), now
stated above; it found no disagreement with the Rust mount.

`python3 tools/fs7_test.py` (host only, no guest) reads the `seed7` fixture and
three full-size images exported by `cargo test -p rustic-fs --test fs7_image`:
all four record states, a removal whose snapshot stays owned, an executed
admission, a three-run noncontiguous file and an epoch advanced by retention
maintenance. It regenerates the payload patterns, compares each view with
`report7` (which now also reports `recovered` and the retained records), and
damages copies of the history image. Both readers refuse a flipped live or
retained-only payload byte, both broken header copies, a broken aggregate named
by the valid newest header (no fallback), a resealed older header with a
non-adjacent sequence or a higher identity watermark, a resealed map leak, a
resealed retained snapshot overlapping a live file, a broken record CRC, a
resealed orphaned node and a resealed record from another epoch. Both refuse an
image truncated inside a live payload, truncated by one unused tail sector or one
sector too long; for those three the Rust refusal is `report7`'s exact-size
precheck, not the mount, whose own `Io` refusal of a missing referenced sector is
covered by `crates/fs/tests/fs7_image.rs`. Both accept a torn newest header
by selecting the complete older generation and reporting recovery, and both
accept a flipped padding byte after a file's last byte. A deterministic sweep of
resealed field perturbations must also produce the same accept/refuse verdict
from both readers. Evidence is written to `artifacts/fs7/evidence.json`.

`python3 tools/v7_corrupt_test.py` boots `terminal-v7` on two damaged disposable
copies of a fresh `seed7` volume. With one ELF payload byte flipped, mount
refuses the volume: the guest does not panic, the file service exits with its
startup status 5 (`Corrupt`), the supervisor's mount job reports status 4, the
shell reports the service unavailable and range reads return `Unavailable`, a
service restart is refused the same way, and `ps`, `mem`, `services` and `exit`
keep working. With the newest header copy's checksum broken, mount selects the
older generation: the guest reads the ELF byte-for-byte at its version and the
manifest at its older, empty version, while the newer manifest version is a
version conflict. The volume digest is unchanged in both cases; mount does not
repair the torn copy. The mount's `recovered` flag is visible only on the host.
Evidence is written to `artifacts/boot/terminal-v7-corrupt/result.json`. This
covers mount-time detection only: an external change after mount is still not
detected by range reads, and damaged metadata other than a header copy, guest
publication faults and repair are not exercised in the guest.

## Evidence boundary

Host codec tests cover byte offsets, independently computed CRCs, checksum-valid
malformed records, temporal states, high monotonic identities, size boundaries
through 512 KiB, and mutually incompatible wire profiles. The focused sparse
host-disk suite has 105 v7 cases covering provision/mount, version-pinned bounded
range reads, retained-snapshot range reads, tracked commit/replay,
durable admission, pollable retry, execute/cancel, pre-write refusals, retained
snapshots, torn headers, 105 admission publication cuts, 103 execute/cancel cuts,
recovery after final-flush errors, and streamed staging (byte-identical media
against borrowed replacement and admission from 0 bytes to 512 KiB, a three-run
fragmented payload, 106 streamed tracked and 105 streamed admission cuts,
reservation, abort, release, stale and foreign-token, two interleaved stages,
retry and refusal cases). The separate `upgrade7` suite exercises
conversion/remount, 18 success/refusal cases, and each publication write/flush
failure on sparse host disks. These host tests do not demonstrate real
device/DMA behavior. Guest evidence is recorded in
`artifacts/boot/terminal-v7/result.json` (build `2338ead14911b84f`): the
host-provisioned 67,216,384-byte image passed read-only service reads for
`file-server.elf` and its manifest in two boots, survived one service restart,
and had the same before/after SHA-256. QEMU's temporary block backend is opened
read/write because the filesystem mount's required initial FLUSH is rejected by
the local QEMU read-only backend; the service grants only the read protocol, and
the full volume digest is checked unchanged. The build ID records the source
tree, toolchain, image and application hashes for this run. CI's boot job
repeats this disposable V7 harness. This read/remount slice does not
establish application execution, full-storage/retention behavior, guest
publication-fault recovery, bounded control latency or complete #51 acceptance.
