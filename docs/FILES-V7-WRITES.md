<!-- SPDX-License-Identifier: Apache-2.0 -->
# V7 tracked writes (profile 2)

In the explicitly selected `mode=terminal-v7` fixture the V7 file service now
accepts streamed profile-2 tracked replacements of up to 512 KiB (#51). Each
write is published by the V7 owner's [streamed stage](WORKSPACE-FORMAT7.md#streamed-staging)
as a `DirectCommitted` retained record, and the client gets a completed-operation
receipt it can check against the bytes it sent. The same receipt can later be
looked up cold by operation ID or retry key, and the owner can revoke the
shell's binding in the middle of a transfer. When the eight-record budget is
`Full`, the owner can explicitly run
[retention maintenance](#owner-retention-maintenance) to reclaim the records
and advance the retry epoch, so useful writes can continue. The v5 service
stays the default and is unchanged.

## What is verified

- Host: `cargo test -p rustic-file-service --test v7_write` runs the real
  `Server7` over a sparse in-memory disk. It covers a 200 KiB write with its
  receipt, SHA-256 and exact bytes; a stale version refused before any write;
  eight records and then `Full`; revocation, detach and expiry during a
  transfer, each leaving no open stage and the file unchanged, after which the
  same retry key is still usable; an exact retry after remount that returns a
  byte-identical receipt with no disk writes or flushes; a differing last byte
  and a differing size, both `IdempotencyConflict`; separate retry scopes per
  subject; refused grant profiles, a read-only grant, an out-of-scope resource
  and a foreign lineage; offset, object and two-stage limits; a resent chunk
  refused with `Offset` while the transfer continues at the right offset; a
  sector write that fails mid-chunk, which drops the transfer (`NoTransfer`
  afterwards), leaves no open stage, fences the volume (`Uncertain` for new
  opens and reads) and publishes nothing; an empty replacement; and receipt
  parts that are visible only to the slot that produced or looked them up. Unit
  tests in `v7/write.rs` check that a committed record the service cannot
  describe or that contradicts the request is reported as `Uncertain`. The same
  target's lookup cases (`tests/v7_write/lookup.rs`) remount after a 513-byte
  and a 300 KiB write and look both up by operation ID and by retry key from
  another slot: each receipt is byte-identical to the commit receipt, the
  SHA-256 is the content's, one snapshot read is made per sector and nothing is
  written. They also cover a record whose file was removed (still visible
  through its workspace), an unknown ID or key (`OutcomeUnknown`), a key in
  another epoch (`ExpiredEpoch`), a foreign lineage (`Lineage`), unmarked
  profile-1 lookups (`Unsupported`), another subject and scopes that contain
  neither the workspace nor the object (`OutcomeUnknown`, with no snapshot
  read), a read-only grant (`Denied`), the workspaces root and the object as
  valid scopes, and revocation forgetting a looked-up receipt.
  The retention cases (`tests/v7_write/retention.rs`) check that
  `Server7::maintain_retention` is `Busy`, with the header, the records and the
  disk writes and flushes unchanged, while an exact retry holds a transfer at
  `Full` (the retry then completes and replays) and while a fresh transfer is
  open on another slot until it is aborted. After eight 1,500-byte writes it
  drops all eight records, frees exactly the 21 sectors of the seven superseded
  snapshots (the live file keeps its own), advances the epoch by one and
  publishes one generation, which a cold mount selects; a second maintenance
  with nothing to reclaim advances the epoch and frees nothing. Over three
  fill, maintain and write cycles, an exact retry, a fresh write and a retry
  lookup naming the old epoch are `ExpiredEpoch`, a lookup of a reclaimed
  operation ID and a receipt part the slot had cached are `OutcomeUnknown`, and
  the same key is a fresh operation in the new epoch. A client packet with the
  administrative opcode changes nothing, and a write that fails during the
  maintenance publication leaves the service `Uncertain` and fenced while a
  cold mount still selects the previous generation.
  `cargo test -p rustic-supervisor --lib retention` checks the job's
  administrative words and which service replies become a job result.
  `cargo test -p rustic-sdk --test file_workspace` checks that the SDK client
  streams from its source, aborts when the source fails, reports a receipt
  that does not match as `Uncertain`, advances a stepwise transfer only on
  acknowledged chunks, aborts an incomplete one instead of committing it, and
  collects a looked-up receipt, refusing one that names another operation.
  `cargo test -p rustic-supervisor --lib shell_binding` checks the revocation
  and grant words of the shell's binding and which replies confirm them.
- Guest: `python3 tools/v7_write_test.py` boots a fresh `seed7 --scratch`
  volume twice. In boot 1 the shell writes six deterministic patterns into
  `scratch.bin` (513 B, 8 KiB, 64 KiB, 1 B, 256 KiB, 512 KiB), which fills the
  eight-record budget next to the two seed records. Before and after those
  writes, a future and a stale version are both refused with `Version`, and one
  more write is refused with `Full`. Before the 512 KiB write, when exactly one
  record slot is left, the same 512 KiB pattern is started under its own key
  and cut after 400 chunks (16,000 acknowledged bytes, so 31 sectors staged as
inferred from the acknowledged bytes), and the owner
  revokes the shell's binding through the supervisor. The next chunk on the old
  endpoint is `Closed`, the same transfer on the adopted binding is
  `NoTransfer`, and a guest range read shows the 256 KiB write's version and
  size. The 512 KiB write then succeeds on the new binding; it needs the last
  record slot, which the revoked stage would otherwise still reserve. A guest
  range read then shows the last write's version and size. After the boot, the
  independent
  [`oracle7`](WORKSPACE-FORMAT7.md#independent-reader-and-damaged-input) reader
  must find one retained record for every printed receipt with the same
  object, previous and committed version, size, epoch, key, instance and
  SHA-256. The live file must be the last pattern, and the application pair
  must be unchanged. It must also find no record for the revoked key, and the
  512 KiB write must name the 256 KiB write's version as its previous version
  and commit the next sequence, so the cut published nothing. In boot 2 an
  exact retry of the 8 KiB write prints the same receipt lines, and retries
  with different bytes or a different size are `IdempotencyConflict`. The
  513-byte and 512 KiB writes are then looked up by operation ID and by retry
  key, and each lookup prints exactly the lines printed at commit, including
  the SHA-256. A lookup of the revoked key is `OutcomeUnknown`. The volume
  digest does not change across boot 2, so neither the replay nor the lookups
  wrote anything. Evidence is written to
  `artifacts/boot/terminal-v7-write/result.json`.
- Guest: `python3 tools/v7_retention_test.py` boots another fresh
  `seed7 --scratch` volume twice and runs three fill, maintain and write
  cycles with 4 KiB patterns, two in boot 1 and one after the reboot. Each
  cycle fills the eight-record budget (six writes next to the two seed records
  in the first cycle, seven later), checks that one more write is `Full`, runs
  `maintain-v7`, writes again in the new epoch (the write names the last
  write's version as its previous version) and then checks the old epoch: an
  exact retry of the cycle's first write, a fresh write naming the old epoch
  and a lookup by the old retry key are `ExpiredEpoch`, and a lookup of the
  reclaimed operation ID is `OutcomeUnknown`. In the first cycle, before the
  maintenance succeeds, an exact retry of the last write holds a transfer open
  after 40 chunks (1,600 bytes) while the owner asks for maintenance: the
  answer is `Busy`, the transfer is aborted, one more write is still `Full` and
  the image digest is unchanged. The independent `oracle7` reader checks the
  image before and after every maintenance while the guest is idle at the
  prompt (the shell prints only after the service has published and flushed,
  and QEMU writes through the host page cache the reader sees): the epoch
  advanced by one, one generation was published, every record was dropped,
  free space grew by exactly the sectors held only by retained snapshots (the
  seed records and the last write alias live files and keep their sectors), and
  the live files did not change. After each clean shutdown the image holds the
  persisted epoch and exactly one record, which matches the last receipt, the
  live file is the last pattern and the application pair is unchanged. After
  the reboot the last write is looked up by ID and by retry key with the lines
  printed at commit, and a key of the previous epoch is `ExpiredEpoch`. Evidence
  is written to `artifacts/boot/terminal-v7-retention/result.json`.

Guest timing from one run under QEMU TCG on the reference machine (build
`4108eab5007c3569`), measured by the shell from before the open to after the
receipt, or around the whole lookup (PIT ticks, 100 per second):

| Operation | Ticks |
| --- | --- |
| Write 513 B | 37 |
| Write 8 KiB | 30 |
| Write 64 KiB | 102 |
| Write 1 B | 2 |
| Write 256 KiB | 292 |
| Write 512 KiB | 558 |
| Lookup 513 B by ID / by retry key (boot 2) | 1 / 1 |
| Lookup 512 KiB by ID / by retry key (boot 2) | 32 / 11 |

Owner retention maintenance, measured by the shell from the job request to its
completed status in one `tools/v7_retention_test.py` run (build
`21c764bced06ebfd`), took 2 ticks in each of the three cycles, which
reclaimed eight records and 40, 56 and 56 sectors. It is one metadata
publication with its flushes and no payload I/O.

Payload cost grows with size, about one 40-byte IPC round trip per chunk plus
one blocking sector write per 512 bytes. The commit's metadata publication and
flushes add a variable cost that depends on the host: the 1-byte write took
longer than the 513-byte one in an earlier run, and in this run the 513-byte
write took longer than the 8 KiB one. A cold
lookup reads the retained snapshot once, one blocking sector read per 512
bytes, and makes three receipt round trips. These are single measurements, not
a benchmark.

## Selection and authority

The file server's ready report is `[0, 2, 256, 524288, 8, 1, 0, 0]`. Word 5
bit 0 means profile-2 tracked writes are served, and the supervisor checks the
whole report exactly. V7 grants accept two rights profiles: read-only (`1`)
with subject 0, or read, write and inspect (`7`) with a nonzero subject. Any
other combination is refused with `Invalid`. The supervisor keeps its own
owner binding read-only and grants the shell profile `7` with subject 2. That
retry scope is separate from the host provisioner's subject 1 seed records, so
the shell can neither replay nor see them. The owner can revoke and reissue
that binding with the supervisor job `REVOKE_SHELL_V7` and run retention
maintenance with `MAINTAIN_V7`; the shell's grant can do neither. See
[V7 write authority](AUTHORITY.md#v7-tracked-write-authority).

Opening a transfer requires write and inspect rights, because the commit reply
is a receipt. The target must be a resource inside the named workspace and the
grant scope, found by walking verified parent links, and the request lineage
must be the volume's. Chunk, commit and abort require write. Lookups and
receipt parts require inspect and a nonzero subject.

## Wire profile

The packets are the existing 64-byte file packets with the profile-2 codecs in
`rustic_abi::files::workspace`:

| Request | Shape | Reply |
| --- | --- | --- |
| `REPLACE_OPEN` (18) | Object in `id`, size (at most 524,288) in `arg`, expected version; 40-byte payload: lineage, workspace, epoch, key, profile marker `2` | Empty acknowledgement |
| `REPLACE_CHUNK` (19) | Object in `id`, byte offset in `arg`, 1 to 40 bytes, contiguous from 0 | Empty acknowledgement |
| `REPLACE_COMMIT` (20) | Object in `id` only | Receipt part at offset 0 |
| `REPLACE_ABORT` (21) | Object in `id` only | Empty acknowledgement |
| `OPERATION_RETRY` (22) | Profile-2 lookup by retry identity: workspace root in `id`, epoch in `version`; 28-byte payload: lineage, key, marker `2` | Receipt part at offset 0 |
| `OPERATION_ID` (23) | Profile-2 lookup by operation ID: sequence in `version`; 20-byte payload: lineage, marker `2` | Receipt part at offset 0 |
| `OPERATION_PART` (24) | Profile-2 lookup by operation ID (20-byte payload), offset 0, 40 or 80 in `arg` | That receipt part |

The receipt is the 104-byte completed-operation receipt with a 32-bit size at
bytes 64 to 67 and the profile marker at 68 to 71. The operation ID and
committed version are the record's commit sequence. The service instance is
the one persisted in the record. The SHA-256 covers the file bytes. Profile-1
(36-byte) opens, unmarked profile-1 lookups and every other mutation or
admission opcode are `Unsupported` on V7.

## Service behavior

`rustic_file_service::Server7` owns the exclusive borrow of the mounted
`Volume7`. The file server keeps that volume in process-static storage. Read
handling (`v7/read.rs`), the grant table (`v7/grants.rs`), scope walks
(`v7/scope.rs`), write policy (`v7/write.rs`), retained-record lookups
(`v7/lookup.rs`) and the per-transfer accumulator (`v7/transfer.rs`) are
separate modules, and `v7.rs` composes them.

- Each slot has at most one transfer. The volume allows two open stages in
  total, so a third concurrent open is `Busy`.
- A transfer holds one 512-byte sector buffer and a running SHA-256. It calls
  `stage_write` whenever the buffer holds `min(512, remaining)` bytes. A fresh
  stage writes the sector, while an exact retry reads the retained snapshot
  sector and compares it.
- The SHA-256 covers the bytes the client supplied. On a retry `finish_tracked`
  succeeds only when every supplied byte equals the snapshot, so the digest is
  the stored content's in both cases.
- A fresh write persists subject = grant subject and instance = the mount's
  first commit sequence (`header.sequence + 1`). Before opening, the service
  looks up a retained record for the same subject, workspace, epoch and key. If
  one exists, the service presents that record's instance, so an exact retry
  after a reboot replays the record instead of being refused.
- Revoking, detaching, expiring or replacing a slot aborts its stage without
  I/O and forgets its receipt. An offset or object refusal leaves the transfer
  open. A stage error ends it: the token is aborted, which is harmless when the
  volume has already ended the stage, and the transfer is dropped. A failed
  payload or publication write fences the volume (`Uncertain`) until restart.
- Once `finish_tracked` has succeeded the effect is committed, so any later
  failure (an unrepresentable record, a record that contradicts the request, a
  receipt encoding error) is answered with status `Uncertain`, never with an
  error that implies no effect. The SDK passes that status through as
  `Uncertain`; an exact retry replays the retained record.
- The last receipt per slot, from a commit or a lookup, is kept in memory for
  `OPERATION_PART`. It is lost on revocation or restart, and a part of any
  other operation is `OutcomeUnknown`: look it up first.

## Receipt lookups

`OPERATION_ID` and `OPERATION_RETRY` find a retained record of the grant's own
subject: by committed sequence, or by workspace, epoch and key. The receipt is
rebuilt from the record, and its SHA-256 is computed by streaming the retained
snapshot through one 512-byte buffer with `Volume7::read_retained_range`, which
trusts the mount-time CRC check of every snapshot. The answers mirror the v5
lookups:

| Case | Status |
| --- | --- |
| No record with that ID, or that key in the current epoch | `OutcomeUnknown` |
| No visible record with that key and another epoch (missing or out of scope) | `ExpiredEpoch` |
| Lineage other than the volume's | `Lineage` |
| Record neither of whose identities (workspace, object) lies inside the grant scope in the live namespace | `OutcomeUnknown` |
| Admitted (unresolved) record | `Busy` |
| Cancelled record, which the completed profile cannot describe | `Unsupported` |
| Grant without inspect right or subject | `Denied` |

The scope check comes before the state, so an out-of-scope record always
gets exactly the answer a missing one would: `OutcomeUnknown`, or
`ExpiredEpoch` for a retry key outside the current epoch. A record whose file was removed stays visible through its
live workspace. The V7 service creates only direct-committed records; the
admitted and cancelled answers apply to records a host provisioner created and
are not exercised by tests. Lookups make no writes or flushes, and a lookup of
a 512 KiB record reads 1,024 payload sectors while other clients wait.

## Owner revocation during a transfer

The supervisor job `REVOKE_SHELL_V7` (owner request `[38, 0, ...]`, V7 profile
only) sends the file service an administrative `REVOKE` for the shell's client
slot 0. The service aborts that slot's open stage without I/O, forgets its
receipt and closes the old endpoint before it replies. Only after a confirmed
revocation does the supervisor rerun the mount's last two phases: it connects a
new channel and grants it the same shell policy (rights `7`, subject 2, the
workspaces root) under a fresh context. The job result carries the same binding
words as a restart, and the shell adopts it the same way. A refused revocation
leaves the old binding. A failure after the revocation (a refused or
unanswered `GRANT`, a failed channel connect or the job deadline) closes both
ends of the unreported channel and marks the supervisor degraded: the shell has
no file binding, further stage and revocation jobs are refused as busy, and
`restart files` is the recovery path, issuing a fresh service incarnation and
binding and clearing the degraded state. The shell reports the failed job as a
service error; any stage the old binding held was already aborted by the
revocation.

The shell's diagnostic form `replace-pattern-v7 ... SIZE cut CHUNKS` opens the
transfer with `Client::workspace_open`, sends `CHUNKS` chunks (fewer than the
whole file), starts the job and waits for it without adopting the new binding.
It then sends the next chunk on the old endpoint, adopts the binding and sends
the same chunk again, and prints
`cut-v7 chunks=N bytes=B job=J old=OUTCOME new=OUTCOME`. It never commits.

## Owner retention maintenance

Fresh writes are `Full` once the eight retained records are used, and nothing
ever makes room implicitly. The owner can start the supervisor job
`MAINTAIN_V7` (owner request `[39, 0, ...]`, V7 profile only) from the shell's
private owner channel with `maintain-v7`. Starting it is the owner's
declaration that clients have resolved the current-epoch outcomes they need:
storage cannot know whether a completed reply was observed. The supervisor
sends the file service the administrative `MAINTAIN_RETENTION` request (`44`)
on its bootstrap channel; no client packet reaches it. `Server7` refuses with
`Busy`, changing nothing, while any client transfer is open, and
`Volume7::maintain_retention` also refuses open stages and unresolved
(admitted) records. Otherwise the volume drops every terminal record, rebuilds
the allocation map from the live files, so only snapshot sectors no live file
owns are freed, and publishes the next retry epoch through the usual
copy-on-write barriers.

The service replies `[0, previous_epoch, epoch, records, sectors]`. The
supervisor checks that the epoch advanced by exactly one and that the counts
are bounded, and completes the job with `[0, file_status, epoch, reclaimed]`,
where `reclaimed` packs the records (low 32 bits) and freed sectors (high 32
bits); a refusal completes the job with only its file status. The shell prints
`maintain-v7 previous=e_... epoch=e_... records=N sectors=S job=J ticks=T` or
the refusal, for example `error: Busy`.

After a maintenance every slot forgets its cached receipt. An exact retry or a
fresh write that names the old epoch is `ExpiredEpoch`, a lookup by an old
retry key is `ExpiredEpoch`, and a lookup of a reclaimed operation ID or a
part of a forgotten receipt is `OutcomeUnknown`. New writes must name the new
epoch; the same key is then a fresh operation. The shell learns the epoch only
from the job's output; the ready report does not carry it, and the harness
passes it to `replace-pattern-v7` explicitly. A failure during the
publication is `Uncertain` and fences the volume until `restart files`, as
for a commit; after the restart the mount selects whichever generation became
durable. A `MAINTAIN_V7` job that fails with status 4 (its deadline passed,
the administrative exchange failed or the reply was malformed) also leaves the
epoch outcome unknown. After a deadline or a failed exchange the supervisor is
also degraded and refuses further owner jobs until `restart files`, whose
fresh service mounts whichever generation became durable. After the restart the owner can learn the durable
epoch from the next `maintain-v7` report's previous epoch, or from whether a
lookup by a known retry key answers or is `ExpiredEpoch`.

The diagnostic `replace-pattern-v7 ... SIZE hold CHUNKS` opens the transfer,
sends `CHUNKS` chunks, runs the maintenance job while the transfer is open,
aborts the transfer and prints
`hold-v7 chunks=N bytes=B maintain=OUTCOME abort=OUTCOME`. It never commits.
At `Full` only an exact retry of a retained write can hold a transfer, because
a fresh one needs a record slot.

## SDK and shell

`Client::workspace_replace(request, size, fill)` (`crates/sdk/src/files/workspace.rs`)
asks `fill(offset, buffer)` for each 40-byte chunk. Neither side buffers the
whole file. The client aborts the transfer if the source fails before the
commit. It collects the three receipt parts and returns `Uncertain` when the
receipt's workspace, resource, retry, previous version, size or SHA-256 differs
from what it sent. Retain the retry identity before calling. The same steps are
public as `workspace_open`, `workspace_chunk`, `workspace_commit` and
`workspace_abort` over a `WorkspaceTransfer`, which advances only on
acknowledged chunks and holds no authority of its own.
`Client::workspace_operation(query)` looks up a retained receipt by operation ID
or retry identity and returns `Protocol` if the answer names another
operation.

The shell command
`replace-pattern-v7 WORKSPACE RESOURCE VERSION EPOCH KEY SEED SIZE` writes
byte `i` = `(SEED*31 + 7*i + i/509) mod 256`, so a harness can recompute it. It
prints the receipt in the same format as `replace-ref` and then
`write-v7 size=SIZE ticks=TICKS`. With `cut CHUNKS` appended it performs the
[owner revocation](#owner-revocation-during-a-transfer) diagnostic instead.
`operation-v7 OPERATION_ID` and `operation-v7 WORKSPACE EPOCH KEY` print a
retained receipt in the same format, followed by
`lookup-v7 size=SIZE ticks=TICKS`. `rustic-volume seed7 ... --scratch` adds an
empty `scratch.bin` to the application workspace and reports its ID, version
and resource. Creating it uses no retained record.

## Limits and pending work

- Only the owner's revocation of the shell's own binding is exercised in the
  guest. Detach and expiry during a transfer remain host-tested, and no guest
  case revokes while a chunk or commit is in flight on the service.
- Lookups recompute the SHA-256 from the medium and trust the mount-time CRC
  check; an external change to the medium after mount is not detected.
- Maintenance relies on the owner's declaration that outcomes are resolved.
  The service refuses only what it can see (open transfers, stages and
  admitted records); a client that has not yet looked up a completed outcome
  loses it. `rustic-volume` does not expose maintenance, and the guest refusal
  for an unresolved admission is untested because the V7 service creates no
  admissions (it is host-tested in `rustic-fs`).
- The shell learns the new epoch only from the maintenance output. No state
  query reports the current epoch to a client.
- Maintenance runs inside the single-loop service like a commit. No guest case
  injects a fault into its publication; that path is host-tested.
- The file server serves one request at a time. A chunk that completes a
  sector issues one blocking write, and a commit runs the whole blocking
  publication while other clients wait. Stage writes are not pollable.
- There are no guest fault-injection cases (torn publication, failed flush) for
  this path. Those failure modes are host-tested in `rustic-fs`.
- The shell's subject is fixed supervisor policy. No other client, utility or
  delegated helper receives V7 write authority.
