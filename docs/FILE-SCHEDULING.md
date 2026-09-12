<!-- SPDX-License-Identifier: Apache-2.0 -->

# Bounded scheduled file execution

The second #47 implementation slice lets a native client schedule an already
durable admission and continue before its file publication settles. The shell
and typed SDK use the same file-service path. One publication owns storage;
authorized live status, stop requests and private owner control remain responsive.
This extends [explicit admission](FILE-ADMISSION-API.md) and
[live control](FILE-ACTIVITY.md), without changing the legacy blocking `EXECUTE`.

## Preparation, scheduling and results

| Fact | Meaning | Survives restart |
| --- | --- | --- |
| Durable `Admitted` | Complete arguments are retained; execution is not scheduled by this fact alone | Yes |
| Activity `queued` | This service has accepted an explicit execution ticket | No |
| Activity `running` / `stopping` / `settling` | Publication or prevention is being driven; the snapshot is not a terminal result | No |
| `cancel_requested=true` | An authorized stop was accepted in memory | No |
| Durable `Cancelled` | The service persisted prevention of the file effect | Yes |
| Durable `Committed` and matching receipt | The replacement has its retained completed result | Yes |

`Client::admission_schedule(id)` and `schedule-admission ID` return an activity
snapshot. A newly accepted ticket replies `queued`, with no pending I/O. It can
start or finish before a slow client reads that reply. The reply is an observation
at request handling time, never proof of completion. There is no later unsolicited
terminal reply to the scheduling RPC. Query activity while work is live, then
durable admission and the completed operation to establish its effect.

A repeated schedule while its ticket exists returns that ticket's current view.
It neither creates a second publication nor transfers ownership to the retrier.
Scheduling a terminal record returns `Unavailable`; inspect its retained status.
An unknown identity during execution may return `OutcomeUnknown`. A missing or
malformed scheduling response is `Uncertain`, and the SDK never blindly retries.
Recover through live/durable queries, using a fresh authorized binding if needed.

## Capacity and authority

The FIFO has at most two tickets, derived from the existing two retained records.
One can be running while the other is pending. Legacy and completed receipts
occupy that same durable inventory and are not eligible new admissions. There is
no extra storage capacity behind the queue: a third durable preparation is refused
with `Full`. Duplicate scheduling consumes no additional ticket.

Scheduling requires current `INSPECT | WRITE`, authenticated peer/context and
the retained subject plus object/workspace scope. Live observation requires
`INSPECT`; requesting a stop requires the separate `CANCEL` right. A cancellation
only client can stop queued or active work without gaining inspect/write access.
The ticket keeps the original caller binding; a subsequent grant cannot revive it.

Before execution, the service checks that binding again and validates the saved
expected file version. Revocation, expiry, detach or a conflicting edit prevents
execution and leads to service-owned durable cancellation when storage can settle
it. This differs from legacy explicit `EXECUTE`, whose version refusal leaves the
record admitted. The current record does not retain a structured reason separating
a stale version from lost authority or a requested stop; all persist prevention as
`Cancelled`. A future logical lifecycle profile must describe that distinction.

A queued stop latches `cancel_requested=true` while its phase remains `queued`
and `io_pending=false`. It becomes durable only when the controller records
prevention. A stop after the publication boundary cannot promise rollback.
Accepted stops survive subsequent caller revocation in the live controller;
they do not survive a crash unless their terminal record was persisted.

## Ownership and transport

[ExecutionQueue](../crates/file-service/src/admission/scheduling/mod.rs) belongs to
one native file-server incarnation. Its inventory, request dispatch and execution
are separate modules. It contains bounded metadata and authenticated scope proofs,
not file buffers or authority minted from retained identifiers.

The ordinary dispatcher refreshes those proofs from the actual volume and grants.
During publication, an exclusive storage borrow prevents namespace changes and
grant issuance. The restricted callback receives only client authority, active
observation and queue state; it can recheck current rights/expiry and service
revoke/detach. It cannot reenter the mutating storage handler. Existing retained
work may join the queue through this view while ordinary storage calls return
`Busy`, including new durable preparation and the old settled query/cancel calls.

Each pass gives private owner control the first opportunity, then retries at most
one reply and handles at most one request per client. Scheduled execution includes
the initiating client in that dispatch. An unread reply blocks only its own slot.
At most one queued publication is driven between ordinary dispatch passes.

Publication and cancellation housekeeping use distinct, non-inlined stack phases.
A native test exposed a file-server page fault when their large metadata workspaces
overlapped; separating those lifetimes fixed the reproduced case without increasing
process stacks. No new dependency, `unsafe`, kernel policy, process quota or on-disk
format is introduced. This validation is not a formal worst-case stack proof.

## Restart and uncertain settlement

The queue is volatile and is discarded on service restart or VM reboot. Mounting,
querying and recovery never schedule pending records. The original retained
service instance remains a historical identity, not the new queue incarnation.
After reviewing durable facts and acquiring current authority, a client can
explicitly schedule an `Admitted` record again. A committed record is never replayed.

Uncertain settlement or an unusable volume abandons remaining volatile tickets.
A scheduling acknowledgement therefore promises admission to this live queue,
not eventual success through arbitrary failures. An absent activity snapshot is
insufficient to infer success, cancellation or no effect. Reconcile retained state
and receipts before deciding whether any explicit continuation is appropriate.

## Native wire and logical profile

ABI version 1 adds opcode 59, `admission::SCHEDULE`, with the existing admission-ID
request framing and activity reply. Activity phase 4 is `Queued`; it cannot carry
the pending-I/O flag. Opcodes 56/57 now also observe/stop scheduled tickets.
Old peers reject unknown opcodes/phases, so use the matching SDK rather than
assuming this additive wire extension is supported. Opcode 58 retains its existing
bootstrap support-vector contract; it does not advertise the full logical catalog.

An actual queued observation maps to service-v1 `queued`, effect `none`; a plain
retained `Admitted` still does not. The existing active and terminal correspondence
applies thereafter. Native admission and completion identities remain distinct.
Full `operations.cancel`, one lifecycle identity, explicit failure/profile semantics,
method-specific discovery and events remain #47/#22/#15 work. This increment does
not claim implementation of the eight-method service-v1 catalog.

## Verification

Run the configured commands in [DEVELOPMENT.md](DEVELOPMENT.md). Local validation
for this increment passed `cargo xtask check`, 58 contract tests, 153 runner tests
and native recovery with 35 groups across 70 VM boots. The ordinary terminal
completed 927 first-phase commands across two boots, with manual/deterministic
discovery parity; native read, operation, activity and discovery validators passed.
Both runs used build `20534fb705f76ced` and kernel SHA-256
`f71adb185044fec171472ed867891bcf8ddd6662e6c8a0a349179022789ffa9d`.
Command counts vary with bounded polling. The completed-operation
validator includes the new scheduling evidence checks; `activity-native` additionally
validates the eight pre-existing explicit-execution groups.

Four new groups in [the native harness](../tools/terminal_support/scheduling_cases.py)
use real shell/utility IPC and held VirtIO completions. They verify:

- A returning initiator, same-client live status, a second ticket and duplicate,
  cancellation-only queued stop, full retention, owner progress and reclamation.
- A discarded scheduling reply, live cancellation, stale-reply rejection and
  independently confirmed durable prevention after reboot.
- Abrupt reboot with active and queued work, unchanged bytes, both records still
  admitted, and fresh explicit scheduling of only the selected record.
- Revocation of a queued executor while another publication finishes; the revoked
  record becomes cancelled, and its payload is not written.

Each group includes a second boot and independent disk inspection. Successful
publications require their own matching receipt. Host tests separately exercise
human edits/version conflicts, expiry/detach/regrant, hidden scopes/subjects,
uncertain settlement and malformed/lost replies. Evidence tests reject missing or
duplicate groups, invalid identities/flags, premature cancellation, mismatched
receipts and claimed automatic replay. Raw serial logs are retained; parsing strips
only the exact complete diagnostic frame that can interleave a shell response.

The subsequent [scheduled failure increment](FILE-SCHEDULING-FAILURES.md) adds nine
native groups for late stops, unread cancellation/result replies, actual service
restart, selected pre/post-publication EIO and combined queue/staging/reply pressure.
The subsequent [authority increment](FILE-SCHEDULING-AUTHORITY.md) adds the native
human-edit and scheduled denial matrix, including foreign subjects and prompt
revocation refusals. The current combined inventory is 49 groups/98 boots; the
counts above describe the original scheduling delivery. The full logical
lifecycle/profile remains #47 acceptance.
