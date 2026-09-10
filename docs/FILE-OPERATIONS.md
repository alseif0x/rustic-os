<!-- SPDX-License-Identifier: Apache-2.0 -->

# Workspace replacements and completed operations

The native file service implements a bounded synchronous profile of **files.replace** and **operations.get**, alongside [files.read](FILES-READ.md). The manual shell and deterministic native client use the same typed SDK. A replacement publishes its file bytes, version and retained operation identity together; a caller can recover the original result by workspace/epoch/key even when it never received the operation ID.

This profile returns completed operations only: succeeded, committed, cancel_requested=false. Volatile transfer acknowledgments are not durable queued/running acceptance. General asynchronous lifecycle, operations.cancel, events, discovery, system.status and MCP/provider adapters remain separate work in #12/#13/#22/#43. The complete eight-operation catalog is not advertised as implemented.

## Workflow and authority

1. Resolve the file and selected workspace with Client::references, then read the observed version and retry epoch with Client::read_range. References identify objects; they grant no authority.
2. Save the workspace, resource, expected version, epoch, a nonzero key and exact replacement bytes before submission. The bound is 0–1,024 bytes for a whole-file replacement.
3. Call Client::replace_file. It stages bounded bytes, submits one COMMIT and collects a complete canonical receipt. It never substitutes another key or automatically resubmits the mutation.
4. After an uncertain result, obtain a fresh binding if necessary and call Client::operation_get with the saved workspace/epoch/key. Lookup by operation ID is also available when the ID was received. Independently read the current file to distinguish the recorded result from a later edit.

The retry namespace is trusted recovery subject + volume lineage + monotonic workspace directory ID + epoch + key. The service obtains the subject from a supervisor-issued grant, never a caller argument. The same key in different workspaces is independent. Selecting a different ancestor workspace deliberately selects a different namespace; callers must retain their original reference. Removed and recreated directories have different IDs. These identities do not resist cloning or rollback of the complete volume history.

The actual peer, current context, expiry/revocation, inspection right and original retained target are checked for every request, including each receipt fragment. New effects additionally require write authority and current membership in the named workspace. A read-only helper may observe an epoch but cannot inspect an operation. A foreign subject cannot retrieve the owner's records. Missing and out-of-scope records both return OutcomeUnknown to an inspection-capable caller, so guessing an ID does not reveal hidden operation existence. An inspection-only grant may recover or replay an existing result without receiving authority for a fresh write.

Lookup resolves the retained record before requiring a live path. Exact object/workspace grants, the owner and a surviving ancestor of the workspace can inspect retained results after file deletion. A grant to an intermediate directory cannot prove historical membership after the object disappears unless it also matches the retained workspace; the service denies that case conservatively. Fresh grants and endpoint contexts are still required after restart. No general multi-user subject store or arbitrary recovery delegation is added.

Identical retained arguments return the original result without a disk write, including after a human edit, deletion or restart. Different bytes, resource or expected version under the same namespace/key return IdempotencyConflict. Comparison uses exact bytes and typed fields, not a hash alone. Missing or expired evidence is OutcomeUnknown or ExpiredEpoch, never permission to invent a fresh key and repeat an uncertain effect.

## Manual terminal use

A receipt-capable terminal volume needs an explicit owner command before the new binding is available:

~~~text
enable-operations
mkdir api-demo
write api-demo/note "before"
ref api-demo api-demo/note
read-ref WORKSPACE RESOURCE - 0 1024
replace-ref WORKSPACE RESOURCE VERSION EPOCH k_000000000000002a "after"
operation WORKSPACE EPOCH k_000000000000002a
operation OPERATION_ID
~~~

Replace the capitalized placeholders with the exact returned tokens; the shell does not expand them. The existing legacy replace/receipt commands retain their separate token syntax. For a binary boundary diagnostic, replace-fill-ref WORKSPACE RESOURCE VERSION EPOCH KEY BYTE COUNT replaces the file with COUNT copies of a byte from 0 to 255; COUNT may be 0–1,024.

The shell prints operation ID, original service instance, state/effect and a receipt containing workspace/resource, previous/new versions, size, epoch/key and SHA-256. The digest describes the retained replacement bytes, not whatever happens to be in the file at lookup time. It is content verification, not publisher authentication.

run lost-operation FILE OTHER is an explicit acceptance utility that replaces FILE with the fixed text reply deliberately unobserved under key 77. Its supervisor grant delegates the owner's recovery subject only for that selected file; OTHER remains denied. The utility submits COMMIT, waits only for endpoint readiness and exits without receiving or decoding the queued reply. Owner lookup and an independent disk reader establish the result. act PID operation-get checks that an ordinary read-capable session cannot inspect that result merely by observing the epoch.

## Identity, capacity and lifetime

| Value | Canonical native text |
| --- | --- |
| Key | k_ followed by 16 lowercase hex digits, nonzero u64 |
| Operation ID | op_ followed by 32 lineage hex digits, _, and 16 committed-sequence hex digits |
| Service instance | si_ with the same lineage/sequence layout |
| Workspace/resource/version/epoch | Existing [stable reference encodings](FILES-READ.md#identity-and-authority) |

The native binding accepts its canonical key encoding. An adapter must preserve it or provide an explicitly specified collision-free mapping; hashing arbitrary token strings into a u64 key is not part of this API.

An operation ID uses its unique committed metadata sequence within one authoritative volume history. The first newly committed logical replacement in a Server incarnation assigns that sequence as the instance identity, atomically in its receipt; later new replacements in that incarnation retain it. Lookup and replay neither allocate a new instance nor adopt an older one. A restarted server assigns a new instance only when it next commits a new logical replacement. Historical receipts retain their original instance across service/VM restart. This is a completed-operation identity, not a new system.status implementation or a promise of identity before the first commit.

The two retained slots are **global to the volume**, shared with legacy receipts. The retry epoch and explicit rotate-receipts action are also volume-wide. Workspace namespacing does not create independent quotas or independent retention policy. A third new tracked effect returns Full before data publication. Rotation must follow owner reconciliation; it atomically advances the epoch before forgetting records and fences older keys. No clock-based eviction, unbounded journal or automatic compaction is introduced.

Staging remains volatile, with the existing two bounded transfer slots. Regrant clears the client's stage; legacy and scoped COMMIT opcodes cannot consume each other's transfer. Busy on OPEN does not cause the SDK to abort a previous transfer. Once COMMIT may have been admitted, interruption, malformed fragments or loss of the final receipt returns Uncertain. Lookup errors describe that observation and do not establish rollback of the preceding write.

## Explicit format upgrade and publication

A legacy volume first needs the [lineage/recovery upgrade](FILE-RECOVERY.md). enable-operations then migrates the receipt-capable volume to header format 3 and recovery magic RUSTREC2. It preserves old records in the legacy namespace: no workspace or operation identity is fabricated for them. Legacy APIs can still retrieve those records. Existing format-2 images are not silently reinterpreted.

A successful migration publishes both metadata banks in format 3, so an old kernel cannot silently select a stale format-2 fallback after the acknowledged upgrade. This is a one-way compatibility boundary. An interrupted upgrade can leave an earlier or later valid bank; explicitly recover and retry with the new implementation. An uncertain migration poisons the live writer. Normal reads never perform this migration, and it does not reformat or erase the volume.

The existing 174-sector layout, inactive data extents and two seven-sector recovery banks remain unchanged. Within a 1,536-byte retained record, bytes 48–51 store the workspace ID and bytes 56–63 the service instance sequence. Bytes 52–55 and 64–511 remain zero; retained content occupies bytes 512–1,535. Both new fields zero mean a legacy record. Decoders reject partial/invalid namespaces, invalid sequence bounds, noncanonical padding and duplicate namespace/key or committed identities.

Data, object metadata, operation record and publication header use the existing flush-ordered atomic replacement path. The service computes SHA-256 from retained bytes when serving a receipt. No second best-effort operation log is appended after commit. The filesystem owns persistence, the service owns authorization and hashing, the ABI owns pure codecs, and the SDK owns transport/collection. No kernel dependency, unsafe boundary or external dependency is added.

## Native wire binding

The file packet remains 64 bytes with 40 payload bytes. Opcodes 18–24 are REPLACE_OPEN, REPLACE_CHUNK, REPLACE_COMMIT, REPLACE_ABORT, OPERATION_RETRY, OPERATION_ID and OPERATION_PART. The private supervisor migration request is owner-only and is not a public service-v1 operation.

REPLACE_OPEN carries resource ID, total size and expected version in the existing fields. Its 36-byte payload holds lineage (16), workspace ID (4), epoch (8) and key (8), with four zero padding bytes. Chunks are bounded and ordered. Lookup accepts either workspace/epoch/key or lineage/operation sequence; subsequent fragments repeat the operation ID and undergo fresh authorization.

The canonical receipt body is 104 bytes: lineage, workspace/object IDs, previous/committed versions, original instance sequence, epoch/key, u16 size, six reserved zero bytes and 32 digest bytes. It is collected at offsets 0, 40 and 80 in fragments of 40, 40 and 24 bytes. Each response repeats total length and committed sequence. The SDK rejects inconsistent offsets, lengths, identity, padding and arguments; no partial receipt is returned as success.

## Reproduction and evidence boundaries

From the pinned Ubuntu environment described in [development](DEVELOPMENT.md):

~~~sh
cargo xtask check
.cache/contracts-venv/bin/python -m tools.contracts operations-check --output artifacts/operations-host.json
.cache/contracts-venv/bin/python -m unittest discover -s tools/contracts/tests -v
python3 tools/boot.py run --mode recovery-test --timeout 60
.cache/contracts-venv/bin/python -m tools.contracts operations-native --evidence artifacts/boot/recovery-test/recovery.json --output artifacts/operations-native.json
~~~

The completed_operations fixture covers ten payload sizes, including empty, SHA-256 block boundaries, binary bytes and 1,024 bytes. Both lookup forms must reproduce each original result: 30 shared logical exchanges. operations-check executes a bounded host fixture; operations-native validates exchanges from actual native UART evidence and records its kernel hash. Neither command establishes the remaining catalog, a live adapter or general asynchronous behavior.

The combined recovery mission now has 15 groups across 30 VM boots. Six new groups exercise the real discarded IPC reply, independent workspace namespaces, read-versus-inspect authority, later edits/deletion/recreation, explicit service restart, VM reboot, quota/epoch behavior and five data/receipt/metadata/header/final-flush EIO boundaries. Native tests use disposable disks and an independent format/content oracle. Pure filesystem tests additionally cut migration and publication writes/flushes, including torn sectors and both visible/durable disk views; service and SDK tests challenge revocation and malformed fragments.

These are bounded QEMU, process/VM-termination and host disk-model tests. They do not prove physical power-loss behavior, adversarial storage integrity or arbitrary acknowledged-sector loss. The isolated executor runs the reviewed driver against a committed candidate tree, separately from direct local execution. Record actual revisions and results; procedures and host mocks are not evidence of guest execution.
