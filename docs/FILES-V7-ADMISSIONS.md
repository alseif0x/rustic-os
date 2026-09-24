<!-- SPDX-License-Identifier: Apache-2.0 -->
# V7 staged admissions (profile 2)

In the explicitly selected `mode=terminal-v7` fixture the V7 file service now
serves profile-2 staged admissions (#51). A client streams up to 512 KiB into
a `Volume7` [admission stage](WORKSPACE-FORMAT7.md#streamed-staging), and ACCEPT
makes it durable as an `Admitted` retained record without touching the file.
The admission is executed later only by an explicit EXECUTE, or cancelled by an
explicit CANCEL. Status, retained observation and the completion receipt stay
addressable by admission ID and retry identity across reboots. The semantics
mirror the v5 direct (unscheduled) admission path, including owner control
while an admission publication is in flight: a revocation before the header
stops it, and an execution stopped that way is recorded cancelled with cause
`AuthorityLost` ([owner control during a publication](#owner-control-during-a-publication)).
Scheduling, live activity and REQUEST_CANCEL are not served on V7. The v5
service stays the default and is unchanged. Tracked writes are described in
[V7 tracked writes](FILES-V7-WRITES.md).

## What is verified

- Host: `cargo test -p rustic-file-service --test v7_admission` runs the real
  `Server7` over a sparse in-memory disk (25 tests). It covers:
  - Admission of a 64 KiB file: the status is `Admitted`, the ID is the
    published sequence, and GET, profile-2 RETRY and observation v2 (cause
    `none`) agree. The file does not change, and a tracked receipt lookup of
    the pending record is `Busy`.
  - EXECUTE: the file becomes the admitted bytes. The completion receipt
    looked up by `OPERATION_ID` carries the request's identity and the
    SHA-256 of the bytes. EXECUTE, CANCEL and GET replays return the committed
    status with no disk writes or flushes.
  - A concurrent tracked write under another key: EXECUTE is refused with
    `Version` without I/O and the record stays `Admitted`. CANCEL then gives
    `Cancelled` with cause `Requested` in observation v2. Cancelled replays
    and RETRY write nothing, and the file keeps the tracked write.
  - An exact admission retry returns the same status without writes. A
    differing last byte or size is `IdempotencyConflict` with no stage left.
    After execution the same retry reports the committed admission.
  - Remount: a new mount still shows `Admitted`. The exact retry presents the
    earlier mount's instance, and execution commits with the matching receipt.
  - Owner maintenance is `Busy` with no I/O while the admission is unresolved.
    After CANCEL it succeeds and the admission is `OutcomeUnknown`.
  - The tracked-write profile (7) can admit and execute, but its CANCEL is
    `Denied`.
  - After the target file is removed, EXECUTE is `Denied` without I/O, while
    GET still shows the admission and CANCEL resolves it. A unit test in
    `v7/admission.rs` shows CANCEL without inspect is `Denied` before any
    lookup (no installable profile holds cancel without inspect).
  - Another subject or an out-of-scope grant sees `OutcomeUnknown` for GET,
    EXECUTE, CANCEL and OBSERVE. A read-only grant is `Denied`. An ID of
    another lineage is `Lineage`, and an unknown ID is `OutcomeUnknown`, all
    without I/O.
  - Read-only and out-of-scope grants cannot stage.
  - Unmarked profile-1 OPEN (36 bytes) and RETRY (24 bytes), SCHEDULE,
    ACTIVITY and REQUEST_CANCEL are `Unsupported`. A malformed ID is
    `Protocol`.
  - Stage kinds never cross. REPLACE_CHUNK, COMMIT and ABORT cannot reach an
    admission transfer, and CHUNK, ACCEPT and ABORT cannot reach a tracked
    one: both are `NoTransfer`. One slot holds one transfer of either kind
    (`Busy`), and an incomplete ACCEPT is `Offset` with the stage kept.
  - A committed direct write's retry identity cannot be admitted
    (`IdempotencyConflict`).
  - Revocation during a staged admission releases the stage. The old context
    is `Revoked`, a fresh binding sees `NoTransfer` and can admit the same key
    afresh.
  - Owner control during a publication (`v7_admission/control.rs`, 12 tests).
    `Server7::handle_with` drives the publication over a pollable disk whose
    every command stays pending for one poll, and the owner callback acts
    between polls:
    - Revocation with EXECUTE's first command outstanding (phase
      `Preparing`) returns `Revoked`. The file keeps its bytes and version,
      the record is `Cancelled` with cause `AuthorityLost` in observation v2,
      a later EXECUTE on a fresh binding replays the cancellation without
      writes, and a remount shows the same. Expiry (`Expired`) and detach
      (`Denied`) at the same point give the same record.
    - Revocation once the header may have been submitted (`Settling`) lets
      the execution settle: the reply is `Uncertain`, the file holds the
      admitted bytes, a fresh binding sees `Committed`, and a remount agrees.
    - Revocation before an ACCEPT's header publishes nothing: no record, the
      same sequence, no stage left, and a fresh binding admits the same key
      afresh. After the header the new admission is retired: `Cancelled` with
      `AuthorityLost`, and the reply is `Revoked`.
    - Revocation before a CANCEL's header leaves the admission `Admitted`;
      after it the cancellation stands (`Cancelled` with cause `Requested`)
      and the reply is `Uncertain`.
    - A disk failure of the `AuthorityLost` prevention after a pre-header
      revocation is `Uncertain`: the volume is fenced, cached receipts are
      forgotten, later requests are refused, and a remount shows the
      admission still `Admitted` with the file unchanged. A drained command
      that completes with an error during the stop gives the same.
    - An exact-retry ACCEPT replays the retained admission without I/O and
      without calling owner control.
    - Revoking another slot during an execution leaves the caller's
      execution committed and reported; the revoked slot's open stage is
      released once the publication has settled, and its next request is
      `Revoked`.
    - Owner control is never called for a request without a publication.

  The file-server library has host tests for what the owner and the other
  clients get while a publication is in flight
  (`cargo test -p rustic-file-server`, `publication.rs`, 4 tests): revocation,
  detach and the readiness probe are served; grants, retention maintenance
  and every other owner request are `Busy`; malformed owner requests keep the
  idle service's refusals; and another client's request is answered `Busy`
  without consuming it.

  The SDK's stepwise client has host tests in `crates/sdk/tests/file_workspace.rs`:
  admission requests only, source failure → ABORT, a foreign-lineage status →
  `Uncertain`, and an admission transfer cannot be committed.
- Guest: `python3 tools/v7_admission_test.py` (also run by
  `python3 tools/boot.py test`) seeds a fresh temporary V7 volume with
  `rustic-volume seed7 --scratch` and boots `mode=terminal-v7` twice. The
  independent reader `oracle7` checks the image while the guest is idle after
  every phase and after each shutdown. Evidence is in
  `artifacts/boot/terminal-v7-admission/result.json`, with `serial.log` and
  `commands-*.jsonl`.
  - Boot 1: `admit-pattern-v7` admits a 65,536-byte pattern. `oracle7` finds
    one `admitted` subject-2 record whose admission number is the published
    sequence and whose staged snapshot owns payload and has the pattern's
    SHA-256; the live file is unchanged. An exact repeat, `admission-v7`
    (retry identity) and `admission ID` print identical lines, and
    `observe-admission-v2` shows `state=admitted prevention=none`.
    `maintain-v7` is `error: Busy`. The image digest is unchanged across these
    steps. After shutdown `oracle7` still shows the admitted record, and
    `report7` agrees on the sequence.
  - Boot 2: `admission ID` prints the same status. `execute-admission`
    commits it, keeping the admission ID and service instance. `oracle7` shows
    `admitted_committed`, and the live file is the pattern.
    `operation-v7 COMPLETION` and `operation-v7 WS EPOCH KEY` print identical
    receipts with the pattern's SHA-256. Execute, cancel and admit replays
    print the committed status with an unchanged image digest.
  - Boot 2, second admission: a 4 KiB admission is overtaken by a 700-byte
    tracked write under another key. `execute-admission` is `error: Version`,
    `admission ID` still prints `admitted` and the image is unchanged.
    `cancel-admission` gives `cancelled`, and `observe-admission-v2` shows
    `prevention=requested`. `oracle7` shows a `cancelled` record with cause
    `requested` and the tracked write as the live file. Cancel, execute and
    retry replays print the same status with an unchanged image.
  - Boot 2, maintenance: with every admission resolved, `maintain-v7` advances
    the epoch and drops all records, which `oracle7` confirms before and after.

  The recorded run on 2026-09-24 (Ubuntu 26.04, QEMU 10.2.1, build
  `dc1e385ada9d369f`) took 8.0 s. The 64 KiB admission took 57 guest ticks,
  its exact retries 46 and 45, and the 4 KiB admission 4. The 64 KiB execution
  took 0.02 s host time, including the UART round trip. Earlier runs of the
  same harness measured 68 and 122 ticks for the first admission, so these are
  single observations, not a baseline. `file-server.elf` is now 401,128 bytes,
  still below the 512 KiB V7 file limit.
- Guest, owner control: `python3 tools/v7_authority_test.py` (also run by
  `python3 tools/boot.py test`) seeds a fresh temporary V7 volume and boots
  `mode=terminal-v7` twice. Evidence is in
  `artifacts/boot/terminal-v7-authority/result.json`, with `serial.log` and
  `commands-*.jsonl`.
  - Boot 1: a 4 KiB admission is executed with
    `execute-admission-v7 ID revoke 0 200`. The shell arms the kernel's
    completion hold for the file service, sends EXECUTE without waiting,
    waits until `io-status` reports the service's first publication write
    held, and starts `REVOKE_SHELL_V7`. The job completes only after
    settlement (212 ticks, the 200-tick hold plus the prevention). The old
    endpoint reports `Uncertain`, and on the new binding the admission is
    `cancelled`, `observe-admission-v2` shows `prevention=authority_lost`,
    and `oracle7` finds a cancelled record with cause `authority_lost`, the
    live file unchanged and exactly one new generation (the prevention, whose
    sequence is the status's terminal). Execute, cancel and status replays
    print the same status with an unchanged image.
  - Boot 1, acceptance: the same diagnostic on `admit-pattern-v7 ... revoke 0
    200` holds ACCEPT's payload flush. The lookup by retry identity on the new
    binding is `OutcomeUnknown`, `oracle7` finds no record for the key and
    the same sequence, and the file is unchanged. The same admission then
    succeeds with the next sequence (the revoked stage released its
    reservation), and executing it commits the pattern through the pollable
    path.
  - Boot 2: the cancelled status and its `authority_lost` cause are unchanged,
    and nothing is published.

  The recorded run on 2026-09-24 (Ubuntu 26.04, QEMU 10.2.1, build
  `f04c38d57527c852`) took 12.9 s. An ordinary 4 KiB execution through the
  pollable path took 0.055 s host time, including the UART round trip (0.036 s
  in an earlier run of the same harness, and 0.02 s for the synchronous path
  above; single observations under varying host load). `file-server.elf` is
  406,096 bytes.

## Wire

All requests use the profile-2 marker where the shape carries one. Status
replies are the v5 `admission::Status` (`arg` 1/2/3 = admitted/cancelled/committed,
`version` = admission number, 32 data bytes). Observations use the v5
observation codecs; version 1 is the coarse projection of version 2.

| Request | Op | Shape | Right |
| --- | --- | --- | --- |
| OPEN | 48 | the 40-byte marked `workspace::Replacement` (`arg` = size) | write + inspect |
| CHUNK | 49 | ≤ 40 bytes at `arg` = offset, `id` = object | write + inspect |
| ACCEPT | 50 | `id` = object only; replies with the status | write + inspect |
| ABORT | 51 | `id` = object only | write + inspect |
| GET | 52 | 16-byte `AdmissionId` | inspect |
| RETRY | 53 | 28 bytes: the retry lookup plus marker 2 | inspect |
| EXECUTE | 54 | 16-byte `AdmissionId` | inspect; write for an admitted record |
| CANCEL | 55 | 16-byte `AdmissionId` | cancel + inspect (the reply discloses the status) |
| OBSERVE | 60 | 16-byte `AdmissionId`, `arg` = 1 or 2 | inspect |

Every request also needs a nonzero retry subject. SCHEDULE, ACTIVITY,
REQUEST_CANCEL and unmarked OPEN/RETRY stay `Unsupported`. The file-server's
ready report sets word 5 bit 1 when admissions are served. The supervisor
requires the exact report `[0, 2, 256, 524288, 8, 3, 0, 0]`.

## Semantics

- **Staging.** Admission transfers share the per-slot transfer table and the
  512-byte accumulator with tracked writes. The stage kind fixed at OPEN
  decides which requests can continue, finish or abort it.
- **Records.** A record with an admission number is an admission; a direct
  tracked write is not. A retry identity that names a direct write refuses a
  new admission with `IdempotencyConflict`, and admission lookups treat such a
  record as missing. As for receipts, only the grant's subject's records
  exist, and a record outside the grant scope is answered as missing
  (`OutcomeUnknown`, or `ExpiredEpoch` for a retry key of another epoch).
- **Exact retries.** An exact retry of OPEN, CHUNK and ACCEPT with the same
  bytes verifies them against the retained snapshot. It returns the record's
  current status (admitted, committed or cancelled) and reuses the instance
  the record persisted, even from an earlier mount. Different bytes or a
  different size are `IdempotencyConflict`.
- **Execution.** The admitting subject is the executor: EXECUTE runs under
  the authority the caller holds when it asks, which must include write access
  to the file within its scope. A file version that changed since admission is
  refused with `Version` before any I/O, and the record stays `Admitted`. This
  is the v5 direct path. Without a scheduler there is no automatic
  `VersionConflict` prevention; the client decides whether to CANCEL. A
  committed execution's receipt is the ordinary profile-2 receipt, looked up
  by the completion ID (`op_…_TERMINAL`) or the retry key. If the target file was
  removed after admission, EXECUTE is `Denied` without I/O (there is no live
  file to write), while the record stays visible through its live workspace;
  CANCEL remains the way to resolve it.
- **Cancellation.** CANCEL of an admitted record publishes `Cancelled` with
  cause `Requested`. CANCEL of a committed record returns the committed status:
  cancellation was too late.
- **Replays.** EXECUTE and CANCEL of a terminal record, GET, RETRY and OBSERVE
  write nothing.
- **Receipt lookups.** `OPERATION_ID` and `OPERATION_RETRY` of an admitted
  record are `Busy`. Those of a cancelled record are `Unsupported`, because the
  completed-only receipt profile cannot describe a cancellation; use GET or
  OBSERVE.
- **Maintenance.** While any admission is `Admitted`, the owner's retention
  maintenance is `Busy` and changes nothing.
- **Publications.** ACCEPT, EXECUTE and CANCEL each drive one pollable
  `Volume7` publication to settlement inside the request. The native service
  polls it over its pollable block adapter and holds the client's reply until
  it settles, with owner control between polls (next section); host callers of
  `Server7::handle` settle it through a synchronous adapter. A disk failure is
  `Uncertain` and leaves the volume fenced until a remount, as for tracked
  writes. A failure to describe an effect that was already published is also
  `Uncertain`.

## Owner control during a publication

At most one publication is in flight: the service serves one request at a
time. Between its polls the file-server gives the administrative channel an
opportunity, then keeps the other clients' transport moving
(`apps/file-server/src/serving/v7/control.rs`, policy in
`apps/file-server/src/publication.rs`):

- **Owner requests.** `REVOKE` takes effect at once: the slot is revoked, its
  endpoint closed and the slot forgotten. Its all-zero acknowledgement is
  queued only after the whole request has settled, including any
  `AuthorityLost` prevention, so the supervisor's `REVOKE_SHELL_V7` job never
  re-grants the shell under an unsettled effect. If that job is cancelled
  (for example by its deadline) with the revocation sent but unacknowledged,
  the supervisor drains the late acknowledgement so it cannot block the next
  owner exchange; the revocation may still have taken effect, and `restart
  files` then issues a fresh binding. Detach and the readiness
  probe are served. `GRANT`, `MAINTAIN_RETENTION` and every other owner
  request are `Busy` with nothing changed. A lost administrative channel
  detaches every client (which stops an unsettled publication like a
  revocation) and ends the service after settlement.
- **Other clients.** Every request from a client other than the publishing
  one is answered `Busy`; nothing is consumed or replaced. Queued replies are
  still retried, and expired slots are closed.
- **Replays.** A publication that is already settled when it starts (an
  exact-retry ACCEPT) has nothing in flight: it is returned without an owner
  opportunity or authority recheck.
- **The caller's authority.** After every owner opportunity, expired grants
  are revoked and the caller's authority for the request (inspect for ACCEPT
  and EXECUTE, cancel for CANCEL) is checked again. Once it is lost:
  - Before the header is submitted, the publication stops
    (`abort_before_header`): an outstanding command drains first, and nothing
    becomes durable. EXECUTE then publishes a second, unstoppable
    cancellation with cause `AuthorityLost`, so the admission can never run
    later. ACCEPT leaves no admission (its staged sectors stay free). CANCEL
    leaves the admission `Admitted`. The reply is the authority error
    (`Revoked`, `Expired` or `Denied`).
  - From header submission on, the publication settles and the effect
    stands. A new admission is then retired with `AuthorityLost` and the
    reply is the authority error; a settled execution or cancellation is
    reported `Uncertain`.
  - A slot that lost its grant during the request drops its stage and
    receipt once the publication has released the volume; it can reach
    neither meanwhile.
- **Latency.** Device completion is not a `WAIT_SET` source. A command still
  pending at a second consecutive opportunity costs one tick of waiting on the
  administrative channel.

Differences from v5, all deliberate:

- The v5 service answers other clients `Busy` (and serves live requests) only
  during an execution, leaving their requests queued during other
  publications. V7 has no live requests and answers `Busy` during every
  admission publication.
- A v5 revocation keeps the slot and rewrites queued replies to `Revoked`; a
  V7 revocation closes and forgets the slot at once, as it always did, so the
  revoked caller's reply is dropped and its old endpoint reports `Closed`
  (`Uncertain` in the SDK for a durable request). The outcome is read on the
  new binding.
- The v5 deferred acknowledgement carries a settlement summary; the V7 one
  stays all-zero, which the supervisor requires, and carries no result.
- There is no REQUEST_CANCEL stop, so `Requested` is never a cause of a stopped
  V7 execution.
- Tracked-write commits (`REPLACE_COMMIT`, `finish_tracked`) and retention
  maintenance stay blocking with no owner control inside them: a revocation
  that arrives meanwhile is served after they complete.

## Authority

The shell's V7 grant is now the admission profile (rights 15 = read, write,
inspect and cancel, subject 2), recorded in
[V7 admission authority](AUTHORITY.md#v7-admission-authority).
The tracked-write profile (7) is still accepted; it can admit and execute but
not cancel.

## Not yet provided

- **Post-header revocation in the guest.** The kernel's hold diagnostic can
  skip at most 16 earlier mutations, and a V7 publication writes 100 metadata
  sectors before its header, so only pre-header revocations are exercised in
  the guest. The post-header cases (settled execution, retired admission) are
  host tests only.
- **Owner control inside tracked commits and maintenance.** They stay
  blocking.
- **No scheduling.** There is no queue, live activity, REQUEST_CANCEL or
  automatic `VersionConflict` cancellation.
- **No guest fault injection yet** on admission, execution or cancellation
  publications. The host tests cover only successful publications and
  refusals before I/O.
- The v5-to-v7 migration of admission history in the guest is still pending.
