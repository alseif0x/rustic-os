<!-- SPDX-License-Identifier: Apache-2.0 -->

# Workspace format 7 and native payload profile 2

Decision for [#51](https://github.com/alseif0x/rustic-os/issues/51), adopted
2026-09-22 under the owner's authorization to continue implementation and
versioned storage/protocol decisions. This increment implements **codecs only**.
There is no v7 mount, publication engine, migration, service backend or capability
advertisement. Production remains v5-backed; the v6 direct probe remains separate.

## Why a successor

The [measured workspace budget](WORKSPACES.md) selects 256 live objects, 64 MiB
of payload, 256 KiB per file and eight retained outcomes. The v6 extent prototype
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
The magic is `RUSTFS3\0`, version 7, layout 1, exact feature mask 7 (extent
payload, immutable snapshots, scoped records). Unknown features are refused.

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
at 262,144 bytes. An empty file has no runs.

The header stores lineage at byte 12, retry epoch at 28, global sequence at 36,
monotonic next identity at 44, and node/map/receipt aggregate CRCs at 48/52/56.
Bytes 60..89 carry the exact geometry/features; other reserved bytes are zero.
The CRC at 508 covers the entire sector with that field zeroed. Epoch and sequence
start at 1; epoch cannot exceed sequence. Next identity starts at 5 and never
means live-object count. `u32::MAX` is the exhausted watermark, not an allocatable
identity. Generation must be 0 or 1.

Nodes store identity/parent/version/length at 0/4/8/16, kind/space/run count/name
length at 20/21/22/23, runs at 24, a zero-padded 32-byte name at 88, payload CRC
at 120 and record CRC at 124. Directory payload is empty. An empty node slot is
all zero. Names retain the existing namespace rules. Per-node decoding does not
establish parentage, namespace validity or global allocation ownership.

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

Other bytes are reserved zero. Lineage is shared through the header. Subject,
workspace, instance, previous version and retry fields are nonzero; object is above the four root
identities and differs from workspace. Contextual validation requires both
identities below the watermark and all sequences within the header sequence.

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
and sibling names, same-space directory ancestry without cycles, and live IDs and
versions below the header watermarks. Retained records must belong to the current
retry epoch, have unique retry identities and event sequences, and agree with any
still-live target's kind and version history. Successful records for the same
object must form a monotonic commit history, and all retained version
observations must be nondecreasing by event sequence. An admitted commit also
requires every retained observation between its admission and commit to see the
same previous version. An unresolved admission may become stale after a later
commit, but a still-live target cannot have advanced past its recorded previous
version before that admission's sequence.

The persisted allocation map must exactly equal ownership by live file extents
and retained payload snapshots. The only shared ownership allowed is one exact
committed snapshot that aliases its current live file version. The caller supplies
the bitmap scratch space, which is cleared before validation and may contain
partial marks on failure. This function receives decoded values: it does not
verify serialized aggregate checksums, read disk, or check payload bytes against
their CRCs. It neither mounts nor publishes a generation.

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

This marker does not negotiate availability. No SDK call, running service,
descriptor or generic service-v1 schema advertises support yet. A later service
integration must explicitly select the new profile and preserve authority,
bounded transfer, conflict, cancellation and recovery semantics. Existing clients
and schema hashes retain their original meaning and 1 KiB limit.

## Required next implementation

The dual-header geometry is not evidence of crash atomicity. A mount owner must
read and decode the selected generation, verify its header-region aggregate
checksums, call whole-generation validation, and check payload integrity; retain
the selected generation's payload until replacement is durable; flush staged
payload and inactive metadata before publishing its header; and fence uncertain
results. Recovery selection must be tested against torn sectors and failed
flushes, not inferred from CRCs.

Allocation must account for both live nodes and retained candidate snapshots.
Shared immutable runs require consistent ownership, and unresolved records must
never be evicted to reclaim space. Admission, honest Full responses, explicit
retention maintenance and pollable cancellation remain unimplemented for v7.
Staging and retry comparison must use bounded buffers, not whole-file copies.

There is no automatic upgrade. A future migration must operate on disposable
copies, preserve or explicitly refuse retained evidence and identity history, and
be tested separately from executable rollback. Never experiment on the owner's
`artifacts/terminal/data.raw`.

## Evidence boundary

Host codec tests cover byte offsets, independently computed CRCs, checksum-valid
malformed records, temporal states, high monotonic identities, size boundaries
through 256 KiB, and mutually incompatible wire profiles. They do not demonstrate
disk I/O, new guest behavior, service integration, bounded control latency or
the consuming workload. Those remain #51 acceptance obligations.
