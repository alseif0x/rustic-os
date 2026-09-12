<!-- SPDX-License-Identifier: Apache-2.0 -->

# Explicit admission API

This native profile exposes durable preparation, status, execution and cancellation
through the file server, SDK and terminal. It builds on [format-4 storage](FILE-ADMISSION.md)
and [controlled settlement](FILE-ADMISSION-CONTROL.md), for #12/#13/#43.

Preparation acknowledges that the complete replacement arguments were retained.
It does not change the file or schedule background execution. The caller explicitly
executes it under current authority, or explicitly uses the subsequent
[scheduling API](FILE-SCHEDULING.md) to return before settlement. This provides a useful
checkpoint for a deterministic client or future agent without silently resuming
old work after restart. It is a bounded native profile, not implementation of the
general service-v1 asynchronous lifecycle or its `operations.cancel` schema.

## State and identity

`ad_<lineage>_<number>` identifies admission. `op_<lineage>_<sequence>` continues
to identify an immutable completed replacement. They are distinct Rust types and
text namespaces. The originating service instance remains a retained fact; it does
not grant authority to the current service or client.

| State returned | File effect | Meaning of another execute/cancel request |
| --- | --- | --- |
| `Admitted` | None | Execute rechecks write authority and expected version; cancel can persist prevention |
| `Cancelled` | None | Both return the existing terminal status without I/O |
| `Committed` | Confirmed | Both return the existing terminal status; cancellation is too late |

For legacy explicit `EXECUTE`, a version conflict leaves the request admitted. The client must cancel it or resolve
the conflict explicitly; it cannot replace the saved expected version in place.
Queries, identical retries, service restart and VM reboot never execute work.
The volume refuses epoch rotation while an admission remains pending. Two records
are shared with legacy/completed receipts, and files remain limited to 1,024 bytes.

The native server processes one controlled publication at a time. [Live activity
and stop requests](FILE-ACTIVITY.md) now remain responsive during explicit execution.
The original settled `CANCEL` and other ordinary storage requests receive `Busy`
from other clients during that execution; public traffic during ACCEPT/CANCEL still
waits for ordinary dispatch. Private owner revocation, expiry and detach remain
serviced between device polls. Only `Cancelled` proves durable prevention; a live
stop acknowledgement does not. Explicit scheduling is available through the separate opcode 59; durable preparation
alone never starts it, and restart never schedules work automatically.

## Authority

| Action | Current grant requirements |
| --- | --- |
| Prepare a new request | `WRITE` + `INSPECT`, trusted nonzero subject, actual workspace/object scope |
| Get status or retry existing exact arguments | `INSPECT` and retained subject/scope |
| Execute an admitted request | `INSPECT` + `WRITE`, retained subject/scope and current expected version |
| Cancel or repeat cancellation | Independent `CANCEL` bit `8`, retained subject/scope |
| Fetch a completion receipt | `INSPECT` through the existing completed-operation API |

`CANCEL` does not imply read, write or inspect authority. Its reply contains only
minimal status and a possible completion identifier, never file bytes or a receipt
hash. The private owner grants the shell all four rights; utilities retain their
previous rights unless explicitly provisioned. Helpers cannot derive additional
rights, scope, lifetime or another subject. Peer/context come from authenticated IPC.

The shared controller rechecks the required right and live binding at each device
poll and before returning the settled result. Lost cancellation authority before
the header drains pending I/O and can abandon the cancellation. After the header,
settlement continues and the caller receives `Uncertain`, not success under a
revoked grant. Admission/execution retain their existing owner-revocation cleanup.

## ABI and SDK

The ABI remains version 1 with new explicitly assigned opcodes. Unknown opcodes,
invalid shapes, reserved bytes and inconsistent replies are rejected. Staged
admission and completed-operation transfers cannot consume or abort each other.

| Opcode | Purpose |
| --- | --- |
| 48 / 49 / 50 / 51 | Admission open / chunk / durable accept / volatile abort |
| 52 / 53 | Status by admission ID / workspace and retry tuple |
| 54 / 55 | Explicit execute / cancel |
| 56 / 57 | Live activity / volatile stop request; separate [activity framing](FILE-ACTIVITY.md) |
| 59 | Schedule a durable admitted record; [queued/activity reply](FILE-SCHEDULING.md) |
| 60 | Read one [coherent observation](FILE-OBSERVATION.md), explicit native profile 1 or cause-aware profile 2 |

Open uses the typed replacement argument layout. ID requests carry 16 lineage
bytes and the admission number in `version`. Retry lookup uses the existing
workspace/epoch/key layout with its own opcode. Status fits one coherent 64-byte
packet: `id=0`, `arg=1/2/3` for admitted/cancelled/committed, `version=admission
number`, `count=32`, and payload `lineage[16]`, `instance:u64`, `terminal:u64`.
Integers are little endian. The remaining eight payload bytes are zero. Terminal
is zero only while admitted, otherwise greater than the admission number.

`rustic_sdk::files::admission` exports the typed identities and status. Client
methods are `stage_admission`, `admit_file`, `admission_retry`, `admission_get`,
`admission_execute` and `admission_cancel`. `Status::completion()` supplies an
optional completed-operation ID for `operation_get`. Staging alone is volatile.
Save the workspace, retry tuple and exact arguments before submitting acceptance.
A lost/malformed durable reply is `Uncertain`; SDK calls never automatically replay
the mutation. Recover acceptance by retry tuple and later transitions by admission ID.

## Manual use

On an explicitly selected disposable development volume, first run
`enable-operations`, then `enable-admissions`. The second command requests the
one-way format-4 upgrade through private owner administration; no disk is upgraded
automatically. Existing format-3 readers must not mount this volume.

1. Create a file and run `ref WORKSPACE PATH` and `stat PATH` to obtain actual references/version.
2. Run `admit-ref WORKSPACE RESOURCE VERSION EPOCH KEY "replacement bytes"`.
3. Use `admission ADMISSION_ID`, or `admission WORKSPACE EPOCH KEY` after a missing reply.
4. Run `execute-admission ADMISSION_ID`, `schedule-admission ADMISSION_ID` or `cancel-admission ADMISSION_ID`. Use `observe-admission ADMISSION_ID` for one [coherent live-or-retained view](FILE-OBSERVATION.md), or `observe-admission-v2 ADMISSION_ID` to include a retained prevention cause; `request-cancel` still requests a live stop.
5. For a committed result, run `operation OPERATION_ID` and verify with `read-ref`.

Copy identifiers returned by the running system. Do not invent a version, epoch,
workspace or authority token. `run lost-admission FILE OTHER` is an owner-launched
acceptance fixture: it submits a real request, waits for reply readiness without
reading the result, exits and leaves recovery to a fresh authorized client.

## Validation and remaining work

Use `cargo xtask check`, the runner/contract tests in [DEVELOPMENT.md](DEVELOPMENT.md),
and `python3 tools/boot.py run --mode recovery-test --timeout 60`. The recovery
inventory at that increment was 23 groups/46 VM boots; [DEVELOPMENT.md](DEVELOPMENT.md) carries the current count. Four new groups use the actual shell, SDK,
authenticated IPC, file server and VirtIO disk: lost acceptance reply plus pending
restart/reboot and explicit execution/cancellation; acceptance first-write EIO;
cancellation first-write EIO; execution final-flush EIO. An independent Python
reader verifies retained states and bytes, including unchanged hashes during replay.

Host checks challenge separate cancellation authority, hidden subjects/scopes,
inspection-only reauthorization, protocol mixing, malformed or lost SDK replies
and every pending cancellation position. Existing storage crash-cut and owner
control tests still apply. These are selected fault models, not physical power-loss
guarantees or an independent security audit. The subsequent [live-control increment](FILE-ACTIVITY.md)
adds native cancellation-only sessions, stops racing real execution and a failed-drain case.

ABI, staging, authorization, publication, SDK and shell concerns have separate
modules. No new external dependency, kernel policy or unsafe boundary is introduced.
The subsequent [scheduling increment](FILE-SCHEDULING.md) supplies a bounded queue
and public live control. Complete failure/profile semantics and coherent events/status
mapped to service-v1 remain lifecycle work. Discovery
and the deterministic M1 mission remain required before claiming the full agent surface.
