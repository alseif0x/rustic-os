<!-- SPDX-License-Identifier: Apache-2.0 -->

# Coherent native operation observation (#47)

`Client::admission_observe(id)` and `observe-admission ID` read one service
observation using the same `ad_...` identity before execution, during scheduling
and after settlement or restart. Clients no longer need to race a live-activity
query against a separate retained-status query merely to find the current phase.
The result is a typed union, not a mixture of fields from different replies.

## Meaning of the response

| Variant | Meaning |
| --- | --- |
| `Retained(Admitted)` | Complete arguments are retained, without a live execution ticket. Prepared work does not automatically resume after restart. |
| `Active(Queued)` | This service incarnation has a ticket. A stop flag is only an in-memory request. |
| `Active(Running/Stopping)` | Execution or prevention is in progress; this is not a terminal file result. |
| `Active(Settling)` | The publication boundary may have been crossed. The outcome is not yet confirmed, so neither rollback nor success may be inferred. |
| `Retained(Cancelled)` | Storage confirms prevention. The existing format does not distinguish a requested stop from a lost guard or version conflict. |
| `Retained(Committed)` | Storage confirms the terminal transaction. `Status::completion()` identifies its immutable completion receipt. |

Live replies contain phase, `cancel_requested` and `io_pending`. Retained replies
contain state and terminal sequence, with no invented historical stop flag.
Both retain the admission ID and originating service instance. The `op_...`
completion identity remains separate; clients keep the original admission ID
when following the lifecycle and may additionally fetch the completed receipt.
Receipt queries and independent file reads retain their existing authority checks.

The observation describes request handling time. Work may advance while a reply
waits in transport. After response loss, a fresh authorized client can query the
same retained identity; this read never schedules, retries a mutation or allocates
a new operation. Unknown or forgotten identities return `OutcomeUnknown`.
An uncertain volume returns `Uncertain` until explicit recovery establishes its
facts. A query error does not establish the outcome of an earlier mutation.

## Authority and ownership

Every observation requires current `INSPECT`, the authenticated peer/context,
and the retained subject/object scope. READ, WRITE or CANCEL alone do not grant
inspection. Revocation and expiry still fence queued replies through the shared
[reply guard](../apps/file-server/src/serving/control/replies.rs).

The server refreshes the bounded retained inventory before ordinary dispatch.
During scheduled execution, its exclusive publication borrow prevents namespace
changes and grant issuance. A small [observation module](../crates/file-service/src/admission/scheduling/observation.rs)
reads that inventory and the live ticket/controller, after fresh authority checks.
It performs no disk I/O and does not change a stop latch or queue ownership.
Completed records and prepared work remain observable while another ticket runs.
For legacy explicit execution, the restricted callback can observe its active
record; it does not expose a general inventory of unrelated work during that borrow.

There is no new storage format, kernel API, unsafe code, dependency, retained slot
or guest process quota. The queue remains owned by one file-server incarnation.

## Native wire profile

Opcode **60 (`OBSERVE`)** uses an explicit observation profile **1**, separate
from file packet ABI version 1 and the logical service-v1 schemas.

The request uses the canonical admission lineage/number layout: `id=0`,
`arg=1` (requested profile), `count=16`, `version=admission number`, lineage in
the first 16 payload bytes and zero padding. An unsupported requested profile
is rejected with `UnsupportedVersion`; older servers reject the unknown opcode.
No fallback issues the legacy two-query sequence or resubmits work.

The reply has `op=60`, `id=1` (profile), and the same context. A 32-byte payload
contains the existing retained-status fields; a 24-byte payload contains the
existing activity fields. The canonical codecs check their state/sequence/flag
invariants and all padding. The new wrapper cannot be decoded as a legacy reply.
The SDK checks the profile and original ID and returns only a complete variant.

This native profile is a foundation for logical lifecycle integration. It does
not advertise general service-v1 `operations.get`/`operations.cancel` support.
Structured retained failure reasons, logical submission/cancellation semantics,
and catalog/profile negotiation remain open. No contract digest or advertised
logical implementation status changes in this increment.

## Verification

Run the commands in [DEVELOPMENT.md](DEVELOPMENT.md). Pure ABI/SDK/service tests
challenge profiles, wrong identities, malformed terminal facts, permission/scope
denials, revocation, unknown records, pending settlement and read-only restart.

The existing recovery mission still has **49 groups / 98 boots**. Its scheduling
groups add **13 coherent native observations**: prepared, active, pending,
stop-requested and retained results, plus unchanged facts after reboot and explicit
resumption. Six comparisons use both the manual shell and an independently running
deterministic client. CANCEL-only and revoked clients receive empty denials.
Matching completion receipts, independent file/record bytes, unchanged read-only
volume prefixes and resource reclamation remain required.

The host evidence checker rejects missing or contradictory observations, aliased
IDs/instances, fabricated confirmation, unsupported profiles, inconsistent client
results and data in denials. These negative fixtures are not guest execution.
Identified execution and publication results are recorded in #47.
