<!-- SPDX-License-Identifier: Apache-2.0 -->

# Terminal operations on V7

This port moves the terminal's existing file operations onto the V7 service.
It does not change file opcodes, packet layouts or the on-disk format. The
owner selected V7 unification on 2026-10-02; the default terminal and its
remaining consumers must be ported before the legacy volume can be removed.
See [current work state](WORK-STATE.md), [storage policy](STORAGE-POLICY.md),
[V7 format](WORKSPACE-FORMAT7.md) and requirement R02.

## Namespace and reads

`LOOKUP`, `STAT` and `LIST` use the existing 40-byte metadata reply, directly
encoded from `Node7`. The length remains a `u32`; converting it through the
legacy `Node` would truncate files above 65,535 bytes. Ordinary `READ` returns
at most 40 bytes, honors a nonzero expected version and reports the exact
current file length/version. The existing references and verified range-read
paths remain available; the shell's `cat` uses them.

Each request checks its envelope, installed peer/context/expiry, shape and
required right before returning metadata or reading payload. Scoped grants
reach their own node and verified descendants. Missing and out-of-scope node
identities share a denial. Looking up a missing name within an authorized
directory still returns `NotFound`.

The workspace shell navigates a virtual root containing only `workspaces`.
The grant's real scope is node 4; listing `/` grants no access to `system`,
`data` or `config`. File-scoped grants cannot browse ancestors through that
virtual root.

The trusted owner's scope zero reaches all four mounted roots. Other live
node scopes, including `data` and `config`, use the same ancestry checks.
The dedicated V7 workspace fixture explicitly requests node 4; it does not
depend on rewriting scope zero inside the service.

`LIST` cursors are ordinals among returned children, rather than V7's physical
node-table positions. The physical successor can be 256, which would wrap
when encoded in the existing one-byte cursor. Four canonical roots occupy
the table, leaving at most 252 workspace descendants, so an ordinal fits.
Iteration ends with a zero-result reply and never wraps to the initial page.
Cursors do not promise a namespace snapshot across concurrent mutations.

## Ordinary replacement mechanism

`CREATE`, `MKDIR` and `REMOVE` require write authority over the live parent or
target and use V7's existing durable namespace mechanism. New object identities
never reuse a removed identity. The four fixed roots cannot be removed or
extended through the virtual root.

`BEGIN` buffers at most the existing 1,024-byte inline limit. Plain replacement
reuses a separate instance of the service's existing transfer pool. The composing
server enforces a combined two-transfer limit across plain and streamed stages,
with one candidate per client. Plain abort cannot discard a streamed candidate.
Regrant, revocation, detachment and expiry clear that client's candidate;
retention maintenance refuses open candidates. An authorized client can abort
its own bound candidate after another client removes the target. An incomplete
commit keeps the candidate; a stale-version commit consumes it without writes.

`Volume7::replace` supplies the storage mechanism for the existing bounded
`BEGIN`/`CHUNK`/`COMMIT` contract. Authorization and candidate ownership belong
to the service. The volume checks the target, size, read-only space and exact
version before I/O; selects copy-on-write runs outside live data, retained
snapshots and open-stage reservations; and publishes using the existing
inactive generation and durability barriers.

Ordinary replacement allocates no durable retry record. A full receipt table
therefore does not prevent ordinary writes when payload capacity remains.
Previously retained snapshots remain intact. A lost reply requires inspection
of the file; it does not gain tracked retry semantics. Publication failures
fence and clear the volume, and remount selects a complete generation.

## Owner authority

The existing owner commands install roots, derive one helper level, add one
disjoint live companion file and revoke a root generation. Any nonzero subset
of known rights is preserved exactly; `READ | WRITE` does not acquire
inspection. Inspection or cancellation requires a trusted nonzero subject.
Helpers inherit that subject, attenuate rights, primary scope and deadline,
and never inherit a companion scope or delegate further.

Replacing or detaching a root fences its whole group. Revoking a helper also
revokes that group; naturally detaching a helper leaves the root and siblings
live. Generation-specific revocation cannot affect a later independent grant
reusing a slot. Validation precedes replacement, so an invalid new grant keeps
the old group intact. Expiration, once observed, cannot be undone by an older
clock value.

Every affected slot loses ordinary candidates, streamed stages and cached
receipts. During an admission publication the loss takes effect immediately;
storage cleanup and the revocation acknowledgement wait for settlement.
Companion-file history and cached receipt parts require that file to remain
live. The existing primary exact scope can still inspect its removed identity.
Endpoint closure and queued-reply cleanup belong to the serving transport.

## Existing tracked and admission profiles

Both existing wire profiles now use the same V7 stages, retained records and
authority. Profile 1 accepts its original 36-byte OPEN, 16-byte ID/part lookup
and 24-byte retry lookup; profile 2 retains its four-byte marker and larger
file bound. These are existing API encodings over one storage format.

An OPEN fixes its transfer's profile and stage kind. The markerless COMMIT
therefore returns the receipt encoding selected by that OPEN. Both receipt
layouts occupy 104 bytes. Profile 1 carries a 16-bit length with its original
1,024-byte bound and zero reserved bytes; profile 2 carries a 32-bit length and
marker 2. A profile-1 query of a larger retained file returns `Size` after
subject/scope checks, before reading the snapshot; it never truncates the
length. Cached parts recheck the live scope and the selected receipt bound.
Admission status replies use the existing shared layout.

Tracked commits now use the same owner-control driver as admission. Their
storage candidate construction is shared with the blocking storage entry
point, and their publication retains exactly 103 commands: 100 inactive
metadata writes, a payload/metadata flush, header write and final flush.
Before header submission, revocation drains outstanding I/O and discards the
candidate without a retained operation. Once the header may have been
submitted, settlement continues and a lost caller receives `Uncertain`;
fresh authorized lookup can recover the committed receipt. Other revoked
members lose their ordinary and streamed candidates after settlement.

The existing two-ticket scheduler and live cancellation interface are ported
to V7; see [V7 admission scheduling](FILES-V7-ADMISSIONS.md#scheduled-execution-and-live-cancellation).
Remaining legacy native consumers and default-volume selection are separate
integration work.

## Recovery compatibility decision

On 2026-10-02 the owner approved retaining `RECOVERY`, `TRACK_BEGIN` and
`RECEIPT` with shared V7 retry history. This is an API projection onto the
existing format, not a disk conversion or a new format. Integration and its
acceptance evidence are tracked in [WORK-STATE.md](WORK-STATE.md).

The older volume could distinguish an unscoped recovery record from a scoped
operation with the same subject, epoch and key. V7 records have no such
discriminator. New recovery records therefore use the target's genuine
top-level directory; no root is repurposed as a hidden namespace. The old
framing searches the subject's retained retry history, returns an identical
direct operation as a replay, and refuses ambiguous or incompatible matches.
Modern scoped APIs keep their workspace-qualified keys. This intentionally
does not preserve the historical separation between those API families.

The projection must preserve the original 1024-byte recovery limit, target
and subject authority, inspection-only replay, expected versions and exact
retry-byte comparison. A competing operation can commit while a candidate is
being staged, so collision checks are required again before publication.
Generic CHUNK/COMMIT/ABORT must remain bound to the transfer family that opened
the candidate. Eight retained records are a mounted service capacity; clients
must not assume the older volume's two-record bound.

## Verification

The focused host suites are:

```sh
cargo test -p rustic-file-service --test v7_namespace --test v7_read --test v7_plain --locked
cargo test -p rustic-file-service --test v7_authority --test v7_admission --locked
cargo test -p rustic-file-service --test v7_profile1 --test v7_write --locked
cargo test -p rustic-file-service --test v7_discovery --test v7_recovery --locked
cargo test -p rustic-file-service --test v7_scheduling --locked
cargo test -p rustic-sdk --test file_binding --locked
cargo test -p rustic-fs --test volume7 untracked --locked
```

They cover metadata, filtering, scope and endpoint denials, malformed packets,
full-table pagination, version-pinned reads, repeated ordinary replacement,
full receipt retention, open-stage reservations, stale stages and publication
cuts, transfer mode collisions, authority cleanup and deletion during transfer.
The disk fixtures model whole-sector writes and flush barriers; they do
not establish physical-media behavior or arbitrary partial-flush durability.

`python3 tools/v7_read_test.py` checks ordinary paths, listing, metadata and
text readback through the production shell, alongside the existing full ELF
reads and staging checks. It compares the complete volume digest before and
after two boots and a service restart.

`python3 tools/v7_plain_test.py` checks namespace mutations, twenty ordinary
writes, removal without identity reuse and exact persistence through reboot
and service restart. The independent Python V7 reader verifies content,
versions, allocation ownership and unchanged retained records. Both harnesses
create fresh temporary images and never use `artifacts/terminal/data.raw`.

`python3 tools/v7_recovery_test.py` checks the original retry/receipt commands
and mounted discovery over two boots. It verifies exact old receipt text,
shared modern-operation replay, ambiguous cross-workspace keys, epoch expiry
and live/retained bytes with the independent reader. Host tests additionally
cover inspection-only replay, a collision introduced after staging began,
wrong-family transfer requests, hidden records, deleted companion authority
and owner revocation with a legacy commit's disk command still pending.

Host checks and harness implementation alone do not establish guest acceptance.
The current checkpoint records the commands actually completed and their
evidence; the overall unification remains pending until its consumers and
recovery tests run on V7.
