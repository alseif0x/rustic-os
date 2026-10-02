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

## Verification

The focused host suites are:

```sh
cargo test -p rustic-file-service --test v7_namespace --test v7_read --test v7_plain --locked
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

Host checks and harness implementation alone do not establish guest acceptance.
The current checkpoint records the commands actually completed and their
evidence; the overall unification remains pending until its consumers and
recovery tests run on V7.
