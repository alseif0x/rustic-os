<!-- SPDX-License-Identifier: Apache-2.0 -->

# Live control of an explicit file execution

The first functional #47 increment lets another authorized native client query an executing admission and request a stop while a real disk command remains pending. It extends the [explicit admission profile](FILE-ADMISSION-API.md). The initiating `admission_execute` call still waits for its settled result; no background execution queue or automatic scheduling is introduced.

## Observable contract

Two new native calls operate only during active explicit execution:

| SDK call / shell command | Authority | Result |
| --- | --- | --- |
| `admission_activity` / `admission-activity ID` | Current `INSPECT`, trusted retained subject and object/workspace scope | One volatile snapshot of the active execution |
| `admission_request_cancel` / `request-cancel ID` | Independent current `CANCEL`, same trusted subject/scope | A stop accepted in memory and a volatile snapshot; not durable prevention |

The snapshot identifies the admission and its originating service instance, with `running`, `stopping` or `settling`, `cancel_requested` and `io_pending`. It contains no file content, receipt hash, device address or new authority. It is an observation at request handling time, not a lease on the current state or a completion result.

- `running`: the file publication has not crossed its header boundary and no stop has been accepted.
- `stopping`: prevention/draining or the durable cancellation record is still in progress. It does not mean cancellation has survived a crash.
- `settling`: the file publication may already be visible; even an accepted stop cannot promise prevention or rollback.

Only the existing durable `admission_get` result `Cancelled` establishes persisted prevention. `Committed` establishes the retained completed result. A failed submitted command remains `Uncertain`, and after restart a request may still be `Admitted`; neither a volatile stop acknowledgement nor missing activity means the effect is known. Remount/query never execute pending work.

After a stop has been accepted under current authority, later revocation does not undo that accepted request. The service drains/prevents publication where still possible and persists its terminal record as owned housekeeping. New requests always recheck authority; losing the executor's binding prevents it from receiving a result under a dead grant. The existing settled `admission_cancel` call retains its separate semantics and authority checks.

No active execution at ordinary dispatch returns `Unavailable` for a valid live request with its required right, without revealing whether the supplied retained ID exists. During active execution an unknown ID or hidden subject/scope returns `OutcomeUnknown`; missing rights, stale contexts and foreign peers remain denied. Ordinary storage requests, including the older settled `CANCEL`, receive `Busy` from other clients while this controller owns storage. Use the new live calls for progress/control, then the durable APIs after settlement. During other controlled transitions such as ACCEPT, public traffic still waits for ordinary dispatch.

## Ownership and progress

[The storage publication](../crates/fs/src/publication/writer.rs) keeps its exclusive volume/disk borrow through settlement. [The active authority view](../crates/file-service/src/admission/active.rs) contains only a bounded observation and prevalidated scope bindings for the four existing client slots. Scope is checked against real objects before borrowing storage. Every live request then rechecks authenticated peer, endpoint/context, current rights and expiry. Issuance and namespace changes require the whole server and cannot occur through this restricted borrow; revoke/detach/expiry remain available.

[The execution controller](../crates/file-service/src/admission/execution.rs) owns transitions and cleanup. [The shared poll driver](../crates/file-service/src/admission/control.rs) advances one storage step at a time and handles terminal/uncertain results. [Native public transport](../apps/file-server/src/serving/control/public.rs) gives private owner control the first opportunity, then retries at most one pending reply and consumes at most one request per other client per pass. A full reply slot cannot prevent other clients or disk settlement progressing. Pending replies are discarded after revoke/detach/expiry; they are not delivered into new grants.

One tick bounds the wait between device polls because block completion is not a `WAIT_SET` source. This is bounded control interleaving within explicit execution, not a general event loop for all filesystem operations. There is one publication, existing two-record retention, no additional process or queue allocation, no disk-format change, no dependency and no new `unsafe` boundary. Rust borrowing is preserved rather than bypassed through raw pointers or recursive calls into the ordinary storage handler.

## Saturation and undrained clients

Capacity is bounded before execution starts: two staging slots, one pending reply per client and the supervisor's two-utility policy. A saturated system must still answer the owner, live status and a stop.

Both staging slots may already be retained by other clients when storage is borrowed. Those transfers neither advance nor disappear during execution; ordinary staging by anyone else is refused with `Busy` both before and during it. A client that never reads its replies keeps its own reply slot full, so the transport skips it and consumes none of its queued requests; its replies remain queued for it and no other client, the private owner path or disk settlement is delayed. Execution and its terminal record are unaffected by that client and its queued work is rechecked against current authority when it finally drains.

Saturating those queues therefore blocks only the client that caused it. It does not extend authority, reorder settlement or turn a volatile stop acknowledgement into durable prevention.

## Service-v1 correspondence

The native live-control profile is not a service-v1 implementation, but its facts must map onto the reviewed `operation` type without lying. The declared correspondence is checked by `python -m tools.contracts activity-check` and, against recorded native evidence, `activity-native --evidence RECOVERY_JSON`:

| Native fact | service-v1 operation |
| --- | --- |
| Activity `running` | `running`, effect `none` |
| Activity `stopping` (stop accepted) | `running`, effect `none`, `cancel_requested` true |
| Activity `settling` | `reconciling`, effect `unknown` |
| Retained `Admitted` | Prepared native work; no scheduled service-v1 state is claimed |
| Retained `Cancelled` | `cancelled`, effect `none` |
| Retained `Committed` | `succeeded`, effect `committed`, with the receipt from the completed-operation API |
| `Uncertain` execution result | `reconciling`, effect `unknown`, whatever the record currently says |
| Refused live request (`Denied`, `Revoked`, `Expired`) | error `access_denied` |
| Hidden identity or scope (`OutcomeUnknown`) | error `outcome_unknown`, effect `unknown` |
| No active execution (`Unavailable`) | error `unavailable`, effect `none` |

An accepted stop is a request, so it never maps to `cancelled`; a settling publication never maps to a rollback; and `Unavailable`/`OutcomeUnknown` keep a retained identity hidden by reporting the caller's own knowledge state rather than whether the work exists.

Preparation does not establish scheduling: the native record retains arguments but does not arrange execution. The checker refuses to turn that fact into `queued`. An uncertain attempt may still map to `reconciling` while its durable admission remains prepared. Successful cases must include their own completed-operation receipt with matching operation and originating instance; a receipt from another test is insufficient. Native identity syntax, integer/flag types, monotonic live phases, unique verified case identities and refusal recovery advice are checked before accepting the correspondence.

One divergence is recorded rather than hidden: pre-terminal work is identified by its admission (`ad_*`) and a committed result by its completed operation (`op_*`), so the native profile does not yet provide a single stable `operation_id` across the lifecycle. An adapter must relate the two. Closing that gap, live discovery and events belong to #47/#22/#43. The [continuation review](H2-CONTINUATION-REVIEW.md) separates implemented tests from the remaining lifecycle work.

## Wire and compatibility

ABI version 1 adds opcode 56 (`ACTIVITY`) and 57 (`REQUEST_CANCEL`). Requests use the existing admission-ID framing. Responses are one 64-byte packet: `id=0`, `version=admission number`, `count=24`, payload `lineage[16]` and `instance:u64`; the remaining 16 bytes are zero. `arg` low bits are 1/2/3 for running/stopping/settling, bit 8 is cancellation requested and bit 9 is pending I/O. All other bits/fields are checked. These packets cannot decode as durable admission status. A missing/malformed stop response is `Uncertain`; the SDK does not automatically replay it.

Old peers reject the new opcodes explicitly. The eight-method service-v1 catalog is still `specified_not_implemented` as a whole; these native live calls do not advertise implementation of its `operations.cancel` schema. Queue admission, full state/profile mapping, discovery and events remain #47/#22/#15 work.

## Native acceptance

The owner can provision two deterministic utilities with `admission-session FILE OTHER RIGHTS`. Rights are explicit bits (`READ=1`, `WRITE=2`, `INSPECT=4`, `CANCEL=8`) on that file; they are not autonomy tiers or authority requested by a model. The supervisor requires its existing explicit-helper policy, issues the trusted recovery subject and retains the existing two-utility limit. `act-admission PID execute|activity|request-cancel ID` starts one owner-stepped SDK action; `actor-status PID` observes it through the independent supervisor channel. This is an acceptance fixture, not live product discovery or a new grant-delegation framework.

[Native cases](../tools/terminal_support/activity_cases.py) hold a real VirtIO completion before publication, during the header and during the final flush. A second CANCEL-only client requests a stop while the owner queries activity/control. The independent disk reader and a second VM boot check the final file and retained result. Further cases deny an inspect-only stop and another file's scope, and inject a failed drain after a live stop request. Helpers cannot execute with only inspect/cancel rights; CANCEL-only inspection is denied. Resource counts return to baseline after normal client cleanup.

Host tests additionally cover every pending publication position, wrong subjects/peers/generations, revoked cancellation authority, uncertain drain failure, malformed framing and a lost/wrong SDK reply without automatic retry. These host models are distinct from native IPC evidence. One further native group and its host model saturate both staging slots and leave a client's replies undrained across a real held execution, then check owner progress, live status, an accepted stop, the durable `Cancelled` record, the retained staging and returned resources. A further native group submits a stop through `act-admission PID lost-stop ID` and never reads its acknowledgement: the service still accepts the stop, prevents the publication and records `Cancelled`, while the undelivered reply poisons only that client's own binding, so its next live call fails instead of decoding a stale result. Client-slot exhaustion beyond the supervisor's two-utility policy, the remaining loss/restart cuts, a background execution queue and full service-v1 conformance remain additional #47 acceptance; the issue stays open.

Run the configured checks in [DEVELOPMENT.md](DEVELOPMENT.md), including:

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode recovery-test --timeout 60
.cache/contracts-venv/bin/python -m tools.contracts activity-check
.cache/contracts-venv/bin/python -m tools.contracts activity-native \
  --evidence artifacts/boot/recovery-test/recovery.json
```

The expanded native recovery inventory is 31 groups/62 VM boots, including six live-control groups, one saturated-execution group and one discarded-acknowledgement group. Results are accepted only with the run's actual kernel/build identifiers, transcripts and independent disk observations. The implementing agent reviews ownership, visibility, dependency direction and the native/host distinction; this is not an independent security audit. Publication and CI evidence are recorded in #47.
