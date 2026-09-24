<!-- SPDX-License-Identifier: Apache-2.0 -->
# V7 staged admissions (profile 2)

In the explicitly selected `mode=terminal-v7` fixture the V7 file service now
serves profile-2 staged admissions (#51). A client streams up to 512 KiB into
a `Volume7` [admission stage](WORKSPACE-FORMAT7.md#streamed-staging), and ACCEPT
makes it durable as an `Admitted` retained record without touching the file.
The admission is executed later only by an explicit EXECUTE, or cancelled by an
explicit CANCEL. Status, retained observation and the completion receipt stay
addressable by admission ID and retry identity across reboots. The semantics
mirror the v5 direct (unscheduled) admission path. Scheduling, live activity
and cancellation during a publication are not served on V7. The v5 service
stays the default and is unchanged. Tracked writes are described in
[V7 tracked writes](FILES-V7-WRITES.md).

## What is verified

- Host: `cargo test -p rustic-file-service --test v7_admission` runs the real
  `Server7` over a sparse in-memory disk (13 tests). It covers:
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
  `Volume7` publication to settlement inside the request, through the
  service's synchronous disk adapter; no other request is served meanwhile. A
  disk failure is `Uncertain` and leaves the volume fenced until a remount, as
  for tracked writes. A failure to describe an effect that was already
  published is also `Uncertain`.

## Authority

The shell's V7 grant is now the admission profile (rights 15 = read, write,
inspect and cancel, subject 2), recorded in
[V7 admission authority](AUTHORITY.md#v7-admission-authority).
The tracked-write profile (7) is still accepted; it can admit and execute but
not cancel.

## Not yet provided

- **Owner control during a publication (6c).** Administrative servicing while
  an admission publication is in flight is not implemented. Revocation,
  expiry or detach therefore cannot interrupt an ACCEPT, EXECUTE or CANCEL
  that has started, and the `AuthorityLost` prevention cause is never produced
  on V7. Admission publications block the single-loop service like commits do.
- **No scheduling.** There is no queue, live activity, REQUEST_CANCEL or
  automatic `VersionConflict` cancellation.
- **No guest fault injection yet** on admission, execution or cancellation
  publications. The host tests cover only successful publications and
  refusals before I/O.
- The v5-to-v7 migration of admission history in the guest is still pending.
