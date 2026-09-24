<!-- SPDX-License-Identifier: Apache-2.0 -->
# V7 tracked writes (profile 2)

In the explicitly selected `mode=terminal-v7` fixture the V7 file service now
accepts streamed profile-2 tracked replacements of up to 512 KiB (#51). Each
write is published by the V7 owner's [streamed stage](WORKSPACE-FORMAT7.md#streamed-staging)
as a `DirectCommitted` retained record, and the client gets a completed-operation
receipt it can check against the bytes it sent. The v5 service stays the default
and is unchanged.

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
  parts that are visible only to the slot that produced them. Unit tests in
  `v7/write.rs` check that a committed record the service cannot describe or
  that contradicts the request is reported as `Uncertain`.
  `cargo test -p rustic-sdk --test file_workspace` checks that the SDK client
  streams from its source, aborts when the source fails and reports a receipt
  that does not match as `Uncertain`.
- Guest: `python3 tools/v7_write_test.py` boots a fresh `seed7 --scratch`
  volume twice. In boot 1 the shell writes six deterministic patterns into
  `scratch.bin` (513 B, 8 KiB, 64 KiB, 1 B, 256 KiB, 512 KiB), which fills the
  eight-record budget next to the two seed records. Before and after those
  writes, a future and a stale version are both refused with `Version`, and one
  more write is refused with `Full`. A guest range read then shows the last
  write's version and size. After the boot, the independent
  [`oracle7`](WORKSPACE-FORMAT7.md#independent-reader-and-damaged-input) reader
  must find one retained record for every printed receipt with the same
  object, previous and committed version, size, epoch, key, instance and
  SHA-256. The live file must be the last pattern, and the application pair
  must be unchanged. In boot 2 an exact retry of the 8 KiB write prints the
  same receipt lines, and retries with different bytes or a different size are
  `IdempotencyConflict`. The volume digest does not change across boot 2.
  Evidence is written to `artifacts/boot/terminal-v7-write/result.json`.

Guest timing from one run under QEMU TCG on the reference machine, measured by
the shell from before the open to after the receipt (PIT ticks, 100 per
second):

| Size | Ticks |
| --- | --- |
| 513 B | 3 |
| 8 KiB | 21 |
| 64 KiB | 122 |
| 1 B | 24 |
| 256 KiB | 258 |
| 512 KiB | 560 |
| 8 KiB exact replay (boot 2) | 9 |

Payload cost grows with size, about one 40-byte IPC round trip per chunk plus
one blocking sector write per 512 bytes. The commit's metadata publication and
flushes add a variable cost that depends on the host: the 1-byte write took
longer than the 513-byte one. These are single measurements, not a benchmark.

## Selection and authority

The file server's ready report is `[0, 2, 256, 524288, 8, 1, 0, 0]`. Word 5
bit 0 means profile-2 tracked writes are served, and the supervisor checks the
whole report exactly. V7 grants accept two rights profiles: read-only (`1`)
with subject 0, or read, write and inspect (`7`) with a nonzero subject. Any
other combination is refused with `Invalid`. The supervisor keeps its own
owner binding read-only and grants the shell profile `7` with subject 2. That
retry scope is separate from the host provisioner's subject 1 seed records, so
the shell can neither replay nor see them. See
[V7 write authority](AUTHORITY.md#v7-tracked-write-authority).

Opening a transfer requires write and inspect rights, because the commit reply
is a receipt. The target must be a resource inside the named workspace and the
grant scope, found by walking verified parent links, and the request lineage
must be the volume's. Chunk, commit and abort require write, and receipt parts
require inspect.

## Wire profile

The packets are the existing 64-byte file packets with the profile-2 codecs in
`rustic_abi::files::workspace`:

| Request | Shape | Reply |
| --- | --- | --- |
| `REPLACE_OPEN` (18) | Object in `id`, size (at most 524,288) in `arg`, expected version; 40-byte payload: lineage, workspace, epoch, key, profile marker `2` | Empty acknowledgement |
| `REPLACE_CHUNK` (19) | Object in `id`, byte offset in `arg`, 1 to 40 bytes, contiguous from 0 | Empty acknowledgement |
| `REPLACE_COMMIT` (20) | Object in `id` only | Receipt part at offset 0 |
| `REPLACE_ABORT` (21) | Object in `id` only | Empty acknowledgement |
| `OPERATION_PART` (24) | Profile-2 lookup by operation ID (20-byte payload), offset 0, 40 or 80 in `arg` | That receipt part |

The receipt is the 104-byte completed-operation receipt with a 32-bit size at
bytes 64 to 67 and the profile marker at 68 to 71. The operation ID and
committed version are the record's commit sequence. The service instance is
the one persisted in the record. The SHA-256 covers the file bytes. Profile-1
(36-byte) opens, profile-2 `OPERATION_ID`/`OPERATION_RETRY` and every other
mutation or admission opcode are `Unsupported` on V7.

## Service behavior

`rustic_file_service::Server7` owns the exclusive borrow of the mounted
`Volume7`. The file server keeps that volume in process-static storage. Read
handling (`v7/read.rs`), the grant table (`v7/grants.rs`), scope walks
(`v7/scope.rs`), write policy (`v7/write.rs`) and the per-transfer accumulator
(`v7/transfer.rs`) are separate modules, and `v7.rs` composes them.

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
- The last receipt per slot is kept in memory for `OPERATION_PART`. It is lost
  on revocation or restart. The exact retry is the recovery path.

## SDK and shell

`Client::workspace_replace(request, size, fill)` (`crates/sdk/src/files/workspace.rs`)
asks `fill(offset, buffer)` for each 40-byte chunk. Neither side buffers the
whole file. The client aborts the transfer if the source fails before the
commit. It collects the three receipt parts and returns `Uncertain` when the
receipt's workspace, resource, retry, previous version, size or SHA-256 differs
from what it sent. Retain the retry identity before calling.

The shell command
`replace-pattern-v7 WORKSPACE RESOURCE VERSION EPOCH KEY SEED SIZE` writes
byte `i` = `(SEED*31 + 7*i + i/509) mod 256`, so a harness can recompute it. It
prints the receipt in the same format as `replace-ref` and then
`write-v7 size=SIZE ticks=TICKS`. `rustic-volume seed7 ... --scratch` adds an
empty `scratch.bin` to the application workspace and reports its ID, version
and resource. Creating it uses no retained record.

## Limits and pending work

- Revocation during a guest transfer (a supervisor job that revokes the shell
  mid-write) and cold receipt lookups of retained records through
  `read_retained_range` are not implemented. They are the next increment.
  Revocation mid-transfer is host-tested only.
- Neither the service nor `rustic-volume` exposes retry-epoch maintenance
  (`Volume7::maintain_retention`), so once the budget is exhausted fresh writes
  stay `Full`. Exact retries still replay.
- The file server serves one request at a time. A chunk that completes a
  sector issues one blocking write, and a commit runs the whole blocking
  publication while other clients wait. Stage writes are not pollable.
- There are no guest fault-injection cases (torn publication, failed flush) for
  this path. Those failure modes are host-tested in `rustic-fs`.
- The shell's subject is fixed supervisor policy. No other client, utility or
  delegated helper receives V7 write authority.
