<!-- SPDX-License-Identifier: Apache-2.0 -->

# Recoverable native replacements

This increment of #12 provides a bounded native replacement and receipt API over real user-mode file services. A caller can recover a committed result after losing its reply, reconnecting to a restarted service or rebooting the VM. The file effect, version and retained canonical arguments publish through the same filesystem header.

This guide documents the legacy subject/volume retry namespace and its storage foundation. The newer [workspace replacement and operation binding](FILE-OPERATIONS.md) adds scoped keys, stable operation/service-instance identities and SHA-256 result receipts without changing the legacy command syntax. Both profiles share the two retained slots and volume-wide epoch. General asynchronous states, cancellation and the complete eight-operation catalog remain separate work.

## Client workflow

1. Observe the file and its version. Obtain the volume lineage/current epoch and choose a nonzero key. Save the complete retry token and exact intended bytes before submission.
2. Stage at most 1024 bytes with that token and expected version. Staging is volatile and has no published effect or durable operation acceptance.
3. Commit. The service checks current authority and resolves a retained key before checking a new operation's version. A retained identical request returns the original receipt without writing, even after a later edit. A changed resource, version, length or byte gives IdempotencyConflict; comparison uses the actual bytes, not a checksum alone.
4. After Uncertain, reconnect with fresh authority and query the original token. A receipt establishes the earlier committed version; read the file independently to observe whether a later edit exists. OutcomeUnknown is not permission to execute again. The SDK never substitutes a fresh key, updates the expected version or automatically replays a mutation.

A failed receipt lookup describes that lookup, not the earlier write. Denied, Revoked and Expired do not imply rollback. A timeout or process exit does not undo submitted storage work. Grant checks serialize at request admission; a revocation arriving during an admitted write waits for it to settle. This is not general cross-service takeover completion.

## Native terminal use

New disks created with tools/terminal.py --initialize include the R0 volume-lineage envelope. For an existing v1 terminal disk, stop its VM and run:

~~~sh
python3 tools/terminal.py --upgrade-recovery
~~~

This explicit one-way upgrade first saves all 174 relevant sectors to artifacts/terminal/data.raw.pre-recovery.bin using exclusive creation and flush. The trusted launcher then provisions a random 128-bit volume lineage in previously reserved sector 1. The native file server publishes the format extension; file identities, versions and extents are preserved. Normal launch never silently upgrades an unprovisioned old disk. Legacy disks retain ordinary file operations and return Unsupported for receipt calls.

The local prefix backup includes all allocated filesystem data in the bounded legacy format; it is not a general 4 GiB block-device backup. Preserve it locally. Do not restore it over later edits or use --initialize to replace a disk. Unknown/malformed provisioning or extension contents are rejected. A torn native upgrade can resume only zeros or an initial extension compatible with the same lineage; it cannot clear published receipt records.

Inside the shell:

~~~text
touch note
stat note
retry-key note 42
replace note EXPECTED_VERSION TOKEN "Hello from a tracked write"
receipt OBJECT_ID TOKEN
restart files
receipt OBJECT_ID TOKEN
~~~

Replace EXPECTED_VERSION and OBJECT_ID with stat output and TOKEN with the entire 64-hex-character retry-key output. Numeric IDs allow owner receipt lookup even after a file is deleted. The SDK uses typed fields; the shell's hex string is just a presentation of 32 encoded bytes.

The existing write command remains the ordinary untracked create/replace operation. Its create-if-absent and replacement are still two commits. Use replace with an explicit observed version/token for recoverable whole-file replacement.

Only two completed receipts are retained per volume. A third new tracked commit returns Full before data writes. Ordinary manual operations remain available. After reconciling the retained outcomes, the owner can deliberately run rotate-receipts. It atomically advances the durable epoch and removes the records. Previously issued tokens then return ExpiredEpoch and cannot be accepted as fresh writes. There is no automatic eviction or clock-based expiration. Identical content under a new accepted key still advances the file version.

run lost-reply FILE OTHER is an explicit destructive-to-FILE test utility: it requires the owner's selected-file read/write/inspection grant, verifies denial of OTHER, stages the fixed text reply deliberately unobserved under key 77, sends COMMIT and exits without reading its reply. It waits only for endpoint readiness, which is not success evidence. The acceptance driver subsequently establishes the actual result with the owner's receipt query and an independent disk oracle. Ordinary reader/watch utilities retain read-only authority and no recovery subject.

## Ownership and identity

The private supervisor-to-file-server grant supplies a recovery subject; a client packet cannot choose it. The local owner is subject 1 and gets explicit inspection authority. The lost-reply fixture is deliberately delegated that subject only within its selected object. Receipt queries, staging, replay and commit all require the authenticated peer/current context and applicable scope. Replays also check the original retained target's visibility. Regrant and restart do not revive old endpoints or generations.

An inspection-only grant may recover/replay an existing result but cannot admit a fresh write or read file content. Another subject cannot inspect the owner's records. Owner receipt lookup can survive object deletion, while new writes still require an existing writable file. The R0 supervisor does not yet provide a general multi-user identity store or arbitrary cross-subject recovery delegation.

The lineage envelope is R0 trusted provisioning, using the host UUID generator, not guest entropy or an authentication secret. The lineage is also stored in the committed extension. Recreating a store must provision a new lineage, so an old token gives Lineage. Native handles still enforce authority; knowing a lineage or key grants none. Restoring an old disk image also restores its evidence and is outside the claimed rollback model. No hostile-host/offline-rollback protection is provided.

## Disk publication

The v1 object banks and extents retain their locations. Version 2 headers keep magic RUSTFS1 and set the u16 format version at byte 8 to 2. Bytes 32–35 hold the CRC-32 of the corresponding recovery bank; bytes 36–511 remain zero. The existing header checksum includes this extension checksum. The two recovery banks occupy sectors 160–166 and 167–173, so the bounded selected area is now 89,088 bytes. Sector 0 and the device's final sector remain untouched.

Each seven-sector recovery bank contains one state sector (magic RUSTREC1, lineage and epoch) and two three-sector records. Each record has a metadata sector and the exact zero-padded 1024-byte canonical payload. It retains subject, epoch/key, object ID, old/new versions and length. The fixed records survive subsequent file changes/removal. Untracked mutations preserve them.

A new tracked replacement performs these steps:

| Step | Publication meaning |
| --- | --- |
| Validate subject/token, retained arguments, capacity and new-write authority/version | No durable change; stale versions and full receipt capacity fail before data writes |
| Write two inactive data sectors; flush | New bytes are not yet published |
| Write seven inactive recovery sectors and four object-metadata sectors; flush | New data/version/receipt remain unpublished together |
| Write the checksummed sequence header; flush | Successful completion acknowledges the joint transition |

Any submitted write/flush failure poisons that mount and returns Uncertain. Recovery selects a valid committed header and its matching checksummed object/receipt banks. Under the tested write/flush model, it observes either the old file without the new record or the new file with that record. CRCs detect corruption; they are not cryptographic authentication or a promise against arbitrary collisions and media loss. Retention uses the same header transition, so a cut preserves either old evidence/epoch or the new epoch that rejects old keys.

## Native wire binding

Packet size remains 64 bytes and existing operation meanings remain unchanged. Added opcodes:

| Operation | Request | Response |
| --- | --- | --- |
| RECOVERY (12) | Target ID; no payload | 24 bytes: lineage[16], epoch u64; arg=2 retained capacity |
| TRACK_BEGIN (13) | Target ID, expected version, total length, 32-byte token | Volatile staging acknowledgement; no queued/running operation claim |
| RECEIPT (14) | Target ID and 32-byte token | Original receipt, or explicit denied/unknown/expired result |

The token is lineage[16], epoch u64, key u64, little endian. CHUNK, ABORT and COMMIT are shared with ordinary transfers. A tracked COMMIT response has id=object, version=committed version, arg=length and a 40-byte payload containing token[32] plus previous version u64. Additional errors are Unsupported=24, Lineage=25, ExpiredEpoch=26, OutcomeUnknown=27 and IdempotencyConflict=28. The SDK also uses Interrupted=29 for an interrupted non-durable foreground wait; potentially durable mutations remain conservatively Uncertain. Rights are read=1, write=2, inspect=4. Private admin grant word 7 carries the trusted subject; private opcode 36 rotates retained evidence. These fields are absent from ordinary file requests.

## Verification and limits

~~~sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode recovery-test --timeout 45
python3 tools/boot.py test --timeout 45
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
~~~

Pure Rust tests cut every replacement write/flush with 0, 1, 16, 32, 256, 511 and 512-byte outcomes and recover both durable and current device views. Separate tests cover retention cuts, resumable upgrades, exact replay after later edits/deletion, different subjects, inspection-only replay, revoked/expired grants and epoch rotation while a replay is staged. These are model tests, not guest execution.

The file-service receipt tests also reproduce the empty second receipt, capacity rejection, epoch rotation and subsequent nonempty replacement sequence. They check the ABI receipt and an independent mount, then cut each write/flush of that final replacement both before and after its possible effect. Each recovered view must contain either the previous empty file or the new content and its receipt together.

The legacy portion of recovery-test runs 18 actual VM boots using the candidate native ELFs. It checks a discarded native IPC reply, fresh authorization after restart, replay after human edits, stale versions, key conflicts, empty replacement, quotas, epoch rotation, separate-VM lookup and legacy upgrade. Five cases inject QEMU EIO at data, receipt-bank, object-metadata, publication-header and final-flush boundaries; each reconciles in a second VM with independent disk decoding. Two additional [admitted-I/O cases](FOREGROUND-CONTROL.md) interrupt observation after a real data-write or final-flush submission and drain during explicit restart. The [workspace operation increment](FILE-OPERATIONS.md) adds six groups and 12 boots, bringing the current combined mission to 15 groups across 30 boots.

This covers selected guest I/O failures and process/VM termination, not physical power failure, every device-cache behavior or arbitrary acknowledged-sector loss. The workspace binding supplies completed-operation identities and SHA-256 receipts; general cancellation, asynchronous lifecycle, independent workspace retention policy, the complete service-v1 catalog and broader takeover cases remain assigned to #12/#13/#22/#43.

The native file-server startup is a separate non-inlined function; its mount/upgrade scratch leaves the stack before the borrowed-state dispatch loop. This addresses a guard-page fault found during native validation. The 64 KiB process-stack budget is unchanged. New code adds no unsafe block or external dependency. The pure volume owns persistence, the file service owns authorization, the SDK owns encoding/client recovery and the host owns provisioning/test VMs. Review is by the implementing agent with automated checks, not an independent audit.

## Unexpected failures and retained evidence

[CI run 34529363684](https://github.com/alseif0x/rustic-os/actions/runs/34529363684) returned an unexpected `Uncertain` for the initial post-rotation replacement at revision `eb1be0f`. The same revision passed the separate isolated recovery mission, and the preceding local direct and isolated missions passed. The failed direct run remains historical evidence in [#46](https://github.com/alseif0x/rustic-os/issues/46); later successes alone cannot identify its cause. Its original UART log cannot distinguish a device failure from a COMMIT transport or receipt-validation failure, and that runner deleted its temporary disk. Subsequent instrumentation and the delayed-device comparison below establish the request-deadline defect with stronger evidence.

Recovery sessions now retain `NAME.commands.jsonl` with a bounded command ordinal, verb, elapsed time and response/acceptance result. Arguments remain in the existing UART transcript, not the timing record. On an unexpected exception, after stopping the owned VM and before removing its disposable disk, the runner saves `failure-NAME.files.bin` (the first 174 sectors), `failure-NAME.last-sector.bin`, and `failure-NAME.json` with hashes, image identity and the original error. Raw capture does not require a valid filesystem bank. A capture error cannot replace the original failure, and no command is replayed to turn the result into success.

These disk bytes describe the state observed after VM shutdown; they do not independently prove that every retained write had completed a flush. The native `sdk-test` image also emits [failure-only block diagnostics](BLOCK.md#failure-diagnostics) before discarding pending request observations. The diagnostic increment itself did not change deadlines, retries, authority or the public `Uncertain` classification. Its follow-up CI captured three real FLUSH timeouts at 25 ticks; the subsequent [device-budget revision](BLOCK.md) raises the per-request allowance to 500 ticks and separates clock-stall detection from aggregate polling speed. The original uninstrumented post-rotation failure cannot be attributed with the same certainty. The earlier `r0-read-v1` measurement archive identifies the read increment's ELF, not these later images.

After a failed service startup, the shell's file client is explicitly unbound (endpoint token zero). File requests now return `Unavailable` before transport admission in that state, while owner control and explicit restart remain available. Fresh binding restores file requests. This check does not relabel malformed replies or poisoned nonzero bindings: their existing `Protocol`, `Closed` and potentially durable `Uncertain` results remain distinct.
