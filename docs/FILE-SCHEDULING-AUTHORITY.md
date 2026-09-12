<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native scheduling authority and intervening edits (#47)

This increment exercises five additional native recovery groups through actual
shell/utility IPC, the file service and VirtIO storage. It supplements the
[scheduling](FILE-SCHEDULING.md) and [failure](FILE-SCHEDULING-FAILURES.md) cases.
Two durable records bound active and pending work; no guest quota or disk format
changes are required.

## Guard matrix

| Native group | Boundary and required result |
| --- | --- |
| Human edit | Admit `first`, write `human-edit` through the shell, then attempt the stale candidate. Explicit execution returns `Version` without changing storage. Scheduling returns an acknowledgement and durably prevents the stale effect; the human bytes/version survive reboot. |
| Read / write | Separate READ-only and WRITE-only clients cannot schedule, inspect live state or cancel either ticket. Refusals contain no operation data. |
| Inspect / combined | INSPECT-only can observe but cannot schedule or cancel. READ + WRITE + INSPECT may observe and duplicate a pending schedule, but cannot cancel. The duplicate keeps the existing ticket. |
| Scope / subject | Two clients have all four rights. One is scoped to another file; the other has its own supervisor-assigned subject. Both receive `OutcomeUnknown` for the owner's retained work, including active/pending control. |
| Revoked / cancel-only | The old revoked client receives `Revoked` during publication. A separate CANCEL-only client cannot schedule or inspect, but can request prevention of both tickets. Both settle as cancelled. |

Every pair case observes actual pending device I/O, verifies unchanged live state
after refusals, and proves owner progress. Denied cancellation leaves the first
authorized write able to commit; the second then fails its expected-version guard.
Accepted early cancellation preserves the original file. Matching native completion
receipts, independent disk bytes/versions, the unrelated file and retained records
are checked before a second boot. Read-only reboot/reconciliation must not rewrite
the selected volume prefix. Resource counters return to their original values.

## Diagnostic authority

`admission-session FILE OTHER RIGHTS [private]` is an owner-issued deterministic
fixture. Existing sessions use the owner recovery subject. With `private`, the
supervisor assigns the new process's PID as its separate diagnostic subject; the
caller cannot supply a subject value. The service still enforces the selected
object scope, rights, generation and authenticated peer. The utility has no
administrative channel or authority to create processes/grants.

This bounded diagnostic identity is not a persistent account or general principal
allocator. It supplies a distinct subject within the tested boot to challenge the
owner's retained records; it does not establish identity continuity across boots.

## Transport defect found and corrected

The first native revoked-client case exposed a real transport defect. During a
publication, polling skipped every revoked or expired binding. The operation
remained protected, but the caller timed out with an uncertain result instead of
receiving the service's refusal.

The [reply guard](../apps/file-server/src/serving/control/replies.rs) removes all
result data from a pending response when the binding is revoked or expired. It
preserves only correlation, opcode, context and the refusal status, including
across backpressure retries. A detached binding drops its response. The live
transport continues its bounded receive/send pass for fenced clients and returns
fresh denials without entering storage or changing execution. Ordinary dispatch
uses the same guard before retrying queued replies.

Pure Rust tests exercise queued secret data, expiry/revocation, repeated retries,
unchanged valid replies and malformed bytes. The native revoked/cancel-only case
reproduces the original failure and verifies progress after correction. No new
`unsafe`, kernel dependency or recursive storage dispatch is introduced.

## Validation and remaining work

Run the procedures in [DEVELOPMENT.md](DEVELOPMENT.md). The combined recovery
inventory is 49 groups / 98 boots; its gate requires 31 held-I/O observations,
91 activity replies and 17 expected uncertainty responses. The contract suite
contains 70 tests and the runner suite 161. The authority validator rejects
omitted/aliased clients, incorrect subjects or rights, missing denials, leaked
operation data, a stop latched by a refused call, false receipts and lost human
edits. Failure capture recognizes these five session names and their reboot
sessions within the existing fixed export budgets.

Publication CI and identified local/native results are recorded in #47. The
complete logical lifecycle remains open: stable operation identity, retained
failure reasons, the actual `operations.cancel` binding, profile/version negotiation
and applicable shared conformance. Native prevention of a stale write does not
yet expose a retained structured `version_conflict` reason through that profile.
