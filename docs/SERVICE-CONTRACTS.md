<!-- SPDX-License-Identifier: Apache-2.0 -->

# Shared service contracts, version 1

The eight-operation specification is checked on the host. [#6](https://github.com/alseif0x/rustic-os/issues/6) and [ADR-0003](architecture/ADR-0003-service-contracts.md) record the contract decision. Native [files.read](FILES-READ.md) and [completed files.replace/operations.get](FILE-OPERATIONS.md) bindings now connect three methods to real storage and authority. The remaining logical surface, agent and MCP integration retain their implementation work; the complete catalog is not a running server or a grant of authority.

## Canonical sources and boundaries

The [catalog](../contracts/services/v1/catalog.json) names eight logical operations. Each adjacent `*.schema.json` owns that operation's input, output, request and response; [shared types](../contracts/services/v1/types.schema.json) own bounded references, bytes, errors and file-operation states. This document owns the behavioral rules that a shape schema cannot establish. Both must change together. Example [messages](../contracts/services/v1/fixtures/messages.json) include invalid forms; [exchanges](../contracts/services/v1/fixtures/exchanges.json) correlate requests with responses and include a read/replace/recover/verify mission.

`tools/contracts` resolves only these local schemas and generates neutral tool descriptors. It never generates permission checks or privileged implementations. `inputSchema` describes parameters, `outputSchema` a successful result, and `responseSchema` the complete success/error envelope. The digest covers the manifest entry plus the recursively expanded input, output, request and response schemas, serialized with sorted keys, ASCII JSON escapes and no whitespace. A consumer must also implement this document's semantic checks. Hashes detect contract drift, not publisher authenticity.

Service schemas use version 1 independently of process ABI 1.0, IPC version 1 and the application manifest version. Unknown versions, methods and fields are rejected. A breaking shape, authority or effect change requires a new service version and conformance review; do not silently reinterpret version 1. Adding future operations changes the versioned coverage inventory and requires matching descriptors. A native binding must pass the same logical vectors before advertising support. Provider/MCP adapters may need schema translation; #43/#39 must test that translation instead of assuming every provider accepts every JSON Schema construct.

The authenticated endpoint/session supplies subject, process incarnation, resource/action grant and revocable context. None is a model-supplied JSON argument. An opaque workspace/resource/operation ID is a reference, never a grant or filesystem path. The file service resolves a stable object within the named workspace generation, avoiding prefix, link and rename escapes. The supervisor issues explicit helper subsets in the same revocable context under [ADR-0002](architecture/ADR-0002-authority-and-delegation.md). Console, GUI, SDK, tools and MCP all reach these checks; a service's block privilege is not the requesting client's file privilege.

## Eight initial operations

These are logical names, not HTTP paths or syscall numbers. All successful results and error envelopes have concrete schemas. Discovery must advertise only implemented contracts; the checked-in catalog's `specified_not_implemented` status is not live capability discovery.

| Operation | Request and result | Enforcing owner / authority |
| --- | --- | --- |
| `capabilities.list` | Nullable cursor and page limit; visible method/version/availability entries and next cursor | Registry; session discovery scope |
| `capabilities.describe` | Method and supported version; availability, exact contract ID/digest and actual upper limits | Registry; same discovery scope |
| `files.read` | Workspace, resource, nullable expected version, offset and length; bytes, file version/size, range hash, EOF and retry epoch | File service; read this object in this workspace |
| `files.replace` | Workspace, resource, required expected version, bounded bytes and epoch/key; tracked operation, eventually a commit receipt | File service; replace this object at the effect boundary |
| `operations.get` | Either operation ID, or workspace plus epoch/key; same operation snapshot | File service; inspect this subject's scoped operation or explicit owner recovery authority |
| `operations.cancel` | Operation ID; cancellation disposition and current snapshot | File service; cancel this operation, independent of inspect/read-file authority |
| `events.read` | Workspace, cursor, page limit and maximum wait; authorized invalidations and next cursor | Event service/source; observe the selected workspace without disclosing hidden objects |
| `system.status` | Selected fields; exactly those fields plus service instance and observation ID | Supervisor; selected system observation scope |

`capabilities.describe` returns an identity and digest for the contract bundle shipped with clients, not arbitrary downloaded schemas inside a 64-byte IPC message. Tools receive the derived schemas from that verified local bundle. A missing or mismatched bundle means unsupported contract; do not follow a descriptor URL or execute unfamiliar instructions. Dynamic third-party schema distribution is future work. Availability is `available`, `degraded` or `unavailable` for a known implemented contract; none grants permission. Unimplemented operations are omitted. Discovery is filtered by session/task and paginated; a future task tool may compose several APIs but cannot acquire extra authority.

The native bindings implement files.read and the completed_operations profile of files.replace/operations.get through typed binary requests and a shared shell/client SDK. REFERENCES resolves already authorized native objects; it does not implement discovery. The completed profile stages volatile bytes and returns only a completed receipt. The separate [explicit-admission API](FILE-ADMISSION-API.md) durably prepares replacements and supports explicit execution/cancellation, but does not implement the general queued/running lifecycle or this operations.cancel schema. #47 owns that bounded lifecycle mapping. The catalog remains specified_not_implemented for the complete eight-operation surface; supervisor lifecycle jobs do not establish the other logical methods.

## Bounds and data meaning

| Value | Version 1 bound and mapping |
| --- | --- |
| Reference, cursor, version, retry epoch/key | 1–64 ASCII letters, digits, `_` or `-`; opaque bounded string in a native binding, not an integer handle |
| Offset / observed file size | JSON integer 0–2^53−1; checked native `u64` within that same domain, including range addition |
| Inline file bytes | At most 1,024 decoded bytes; canonical padded RFC 4648 base64 in JSON, byte slice plus length natively |
| Pages | Caller selects 1–16 items; replies cannot exceed that requested limit |
| Event wait | 0–1,000 ms; zero is a poll, timeout without an event is a successful empty page |
| Logical JSON envelope | At most 32,768 serialized bytes, depth at most 16; no duplicate keys, floats, NaN, infinities or invalid Unicode |
| File operation capacity | At most two live replacements per context and 64 retained records per workspace in the first implementation; lower advertised capacities are allowed, never unbounded allocation |

The host checks include decoded byte limits and canonical padding: JSON Schema `maxLength` counts characters and `contentEncoding` alone does not validate decoded data. Read ranges can cover binary or text files. UTF-8 presentation is the tool/client's explicit conversion, never an implicit filesystem requirement. Offsets are bytes. A read starting at EOF returns empty bytes and `eof=true`; a start past EOF is `invalid_request`. A non-EOF result must make progress. The hash covers exactly returned bytes, not an unseen full file. To read several ranges consistently, pin subsequent reads to the first observed version; a change yields `version_conflict`, not mixed versions.

The first replacement replaces the **entire** file with at most 1,024 bytes, including an empty file. It is not append, patch or a large upload. Even identical new content advances the version on a fresh accepted key. A successful receipt includes the old/new versions, resource identity, size, content SHA-256 and retry key. The client verifies the committed version and bytes with `files.read`; a concurrent later edit is a conflict to observe, not evidence that the earlier receipt was false.

Larger file operations require a separately specified bounded transfer handle or stream. They must not be smuggled into unlimited JSON, split into unrelated whole-file replacements or loaded into the model context by default. A future transfer binds its owner/context, service incarnation, total length, offset and quota; incomplete staging has no published file effect. This version reserves no public upload API. #12 supplies basic native file methods; #22 extends the real service to the complete mission before advertising the eight-operation surface.

## Commit, receipts and retries

The file service owns object resolution, version ordering, current authority validation, idempotency records and commit. The adapter cannot establish any of these. After authenticating and checking receipt visibility, first resolve an existing key: identical canonical arguments return the same operation/receipt, even when its own committed version has advanced. Different arguments under that key give `idempotency_conflict`. Only a new key proceeds to admission and the serialized authority/version/commit boundary. Compare decoded bytes and typed fields, not JSON member order, whitespace or a hash alone.

Key scope is **trusted recovery subject + workspace generation + retry epoch + key**. The recovery subject comes from trusted policy and can survive a process restart; that does not restore its old grants. A helper has only explicitly permitted operation visibility. `operations.get` accepts the epoch/key form because the very first response, including its operation ID, can be lost. A guessed ID/key never reveals another subject's record. Reauthorization is required for lookup and replay after restart or regrant; lookup authority does not grant another write.

Before acknowledging `queued` or `running`, reserve and durably record a bounded operation slot. A completed replacement must commit data, version and its receipt as one recoverable local transition. `succeeded` is emitted only after that transition meets the documented persistence guarantee. The [native completed-operation profile](FILE-OPERATIONS.md) uses atomic format-3 publication and does not acknowledge queued/running. A metadata/data write followed by an unrelated best-effort log is insufficient. Inability to reserve evidence is `quota_exceeded` before changing the file. General durable pending states remain future work.

Retention is count-bounded, not an unbounded log or an assumed reliable wall clock. The service durably advances the workspace's retry epoch **before** forgetting completed records, preserves pending records, and never admits an unseen key from an old epoch. Retained old records may still be queried/replayed without execution. After compaction, an old key yields `expired_epoch`/`outcome_unknown`; it is not a new operation. Epochs and file versions cannot repeat within a workspace generation after service/VM restart. New workspace identity is required after recreating/restoring a store whose lineage cannot be preserved. Host/offline rollback resistance is not claimed.

After a lost reply, query by operation ID or epoch/key, then verify the file. While a key is retained, retrying exactly those arguments can return the same operation after current authorization. If evidence has expired, reconcile through observed resource state and owner intent; never silently invent a new key and repeat the effect. A timeout, broken connection or missing receipt is not proof of failure. External network/build/activation effects will need their own reconciliation contracts: this local file rule makes no universal exactly-once promise.

## Progress, cancellation and invalidation

| Snapshot | Known file effect | Client interpretation |
| --- | --- | --- |
| `queued` / `running` | `none` published yet | Accepted/pending; progress is qualitative, no invented percentage |
| `succeeded` | `committed` with receipt | Verify the resource; immutable terminal outcome |
| `failed` | `none`, with failure code | Definitively ended without publishing this replacement |
| `cancelled` | `none` | Definitively stopped before publishing this replacement |
| `reconciling` | `unknown` | Recover/query; do not infer success, failure or permission to repeat |

Normal transitions are queued → running → succeeded/failed/cancelled; queued may fail/cancel without running. An interruption can require reconciling; durable recovery resolves it to a known terminal result or keeps it unknown. Terminal receipts do not change because a client disconnected. After restart, uncommitted work cannot resume under a dead context: fence it, establish its effect, and fail/cancel known uncommitted work. Any fresh execution requires explicit current authority. Receipts retain their original service instance; current status reports the new instance.

`cancel_requested` is separate from final state. `requested`/`already_requested` acknowledges the request, not a rollback; `too_late` includes an already committed replacement. At the serialized boundary, either cancellation/revocation prevents commit or the earlier admitted effect must settle and be reported. A request that races with commit can still end succeeded. This operation never advertises a successful partially replaced file. Partial read ranges, lost/truncated replies and unknown storage completion remain explicit; future multi-effect operations must introduce their own partial-result schema rather than reusing `failed/none`.

Revocation has requested, fenced and effects-settled phases under ADR-0002. Recheck current authority and expected version at each admitted effect, serialize invalidation against commit, reject queued work from the old context, and track already submitted I/O. A moved handle retains its context; regrant creates a fresh context and cannot revive old requests. Preserve owner control progress separately from bulk data; an unresponsive service means incomplete settlement, not a falsely completed takeover. Initial #44/#12 fixtures can use trusted bootstrap before #13, without claiming product policy enforcement.

## Errors and observation recovery

Errors have a stable code, `effect` and `next_action`. Error effect describes this invocation; a denial of a receipt lookup says nothing about whether the earlier write happened. Safe error messages do not echo file data or hidden paths. Authorization precedes existence disclosure; `not_found` is allowed only inside an already authorized scope. For a missing operation record, use `outcome_unknown` instead of `not_found`.

| Error | Expected recovery advice |
| --- | --- |
| `invalid_request` | `fix_request`; no effect |
| `unsupported_version` | `refresh` supported contract or `stop`; no effect |
| `access_denied`, `read_only` | `stop`; no added authority from retry or confirmation |
| `not_found` | `refresh` authorized scope or `stop` |
| `version_conflict` | `refresh`; observe and decide again, never overwrite blindly |
| `idempotency_conflict` | `fix_request` or `stop`; do not reuse the key with different arguments |
| `quota_exceeded` | `retry_same` within a bounded client budget, or `stop`; no admitted effect |
| `unavailable`, `io_error` | `retry_same`/`stop` only for known no-effect; otherwise `reconcile`/`stop` |
| `expired_epoch`, `outcome_unknown` | Unknown earlier outcome; `reconcile`/`stop`, never blind replay |
| `cursor_expired` | `refresh` observation; no mutation effect |

Transport closure, malformed/truncated replies and deadlines are local client observations, not invented success/error responses from the service. Discard an incomplete message; classify an in-flight mutation as unknown and query its key. No implicit retry loops are part of these schemas.

Events are invalidations, not durable receipts or a history of secret contents. Version 1 uses a bounded ring of at most 64 events per workspace, opaque cursors bound to session/context generation, filter and service instance. A null cursor establishes the current head and returns no historical backlog. Establish that head **before** observing resources, then consume changes and re-observe affected state. Pages may repeat after reconnect; consumers deduplicate by event ID within the instance. If retention, restart or authority/filter changes invalidate continuity, return `cursor_expired` and repeat head/observe/drain. Never silently skip an overflow. Recheck visibility when emitting each page; revocation must not expose buffered hidden events. A separate registry cursor similarly expires on visibility/catalog changes.

Observation IDs establish correlation within one service incarnation, not a globally atomic snapshot or synchronized clock. `system.status` returns precisely the requested fields; each is an observation whose freshness can change. No fabricated healthy fallback when a service is absent.

## Native control/data boundary

The existing [IPC](IPC.md) has a 64-byte payload; the [block driver](BLOCK.md) transfers 512-byte sectors. No complete inline JSON request or sector is assumed to fit. The service envelope is a logical contract; IPC supplies authenticated sender and transport correlation. #44 specifies the implemented [block syscall encoding](BLOCK-ACCESS.md); #12 owns the file-service transfer encoding. Service protocol IDs/versions remain separate from the IPC framing; resource IDs resolve to service objects while kernel handles remain owner-checked native values and are never synthesized from strings.

The implemented [block bridge](BLOCK-ACCESS.md) selects copied asynchronous submission/result collection over chunked IPC. Each write snapshots 512 bytes before admission returns; read collection validates its complete output range before consuming a result. Owner-bound scoped handles, two outstanding records/one active device request, cancellation effects and reset-confirmed DMA lifetime are tested with a separately linked ring 3 program and control survivor. The two-VM disk oracle verifies persistence. DMA remains in the kernel; the logical file envelope and IPC limits are unchanged. File-client authority and file-operation receipts remain service responsibilities.

The file protocol in #12 binds transfer state to the same authenticated context and checks lengths/offsets before copying, including out-of-order chunks if selected. A control message must identify protocol/version, correlation and bounded transfer reference; raw pointers never cross service address spaces. Control-plane cancellation/owner progress cannot depend on an unlimited data queue. Split native Rust modules by transport encoding, typed client API, service state and storage; share pure contracts through an implementation-independent crate only when the actual native boundary needs it. The kernel cannot import the SDK, filesystem policy or host schema tools.

The [implemented read binding](FILES-READ.md) uses native opcodes 15–17 within the existing 64-byte file packet. Workspace/resource text identities encode persistent lineage and monotonic directory/object IDs; fresh endpoint/context authority is required separately. OPEN returns a version, size, range SHA-256 and epoch; individually authenticated chunks remain pinned to that version. The SDK checks the full assembled hash and clears caller output on failure. No persistent read lease is introduced. A read-only helper may observe the epoch without acquiring the native receipt-inspection right. [Completed replacements](FILE-OPERATIONS.md) now bind mutation/retry identity to the trusted subject, volume lineage, workspace and epoch/key. Retention remains two shared records and a volume-wide epoch; independent per-workspace retention is not implemented.

## Coverage and next implementations

| Product area | First contract / remaining owned work |
| --- | --- |
| Discovery and observations | Eight-operation v1 catalog; live registry/tools #22, supervisor #13, events #22 |
| Files and workspaces | Bounded read/replace/receipts here; block bridge #44, directory/create/delete/basic storage #12, complete deterministic mission #22 |
| Processes and services | Existing process/IPC/SDK ABI; supervision/lifecycle product methods #13/#14, descriptors #22 |
| Authority and helpers | ADR-0002; real issuance/revocation/receipt visibility #13, owner/pilot interaction #24 |
| Configuration/adaptation | Domain-specific inspect/apply and truthful degradation #38; no frozen generic configuration schema |
| Networking and egress | Device/stack/DNS/TLS #36/#16/#17; endpoint/provider selection and credential use/export remain distinct grants |
| Desktop / native apps | Semantic actions and selected visual capture #40; accessibility coverage is explicit |
| Browser | Navigation, selected DOM/content and actions #41, local engine #7/#19; page content remains untrusted |
| Builds and candidates | Runner channel #42, candidate/test/activate/recover #25/#26/#27; build authority does not imply activation |
| Agent / MCP | Deterministic comparison #43, guest tools #22, integrated pilot #23, interoperable adapter #39 |

Every included native product action needs an API, suitable tool and verification in its owning delivery. These eight methods are the first mission's denominator, not complete v0.1 coverage. Third-party programs may offer API, accessibility or visual access only; report that boundary. Read authority never grants export to an inference provider. A tool presents selected bytes as untrusted content, with explicit egress and credential handling; optional screen access has its own scoped grant. Automatic mode and confirmation add no rights.

## Reproduction and evidence limits

Use Ubuntu 24.04 amd64 / Python 3.12. The host validator is pinned separately from the guest toolchain:

```sh
python3.12 -m venv .cache/contracts-venv
.cache/contracts-venv/bin/python -m pip install --require-hashes --only-binary=:all: -r tools/contracts/requirements.txt
.cache/contracts-venv/bin/python -m tools.contracts check --output artifacts/service-contracts.json
.cache/contracts-venv/bin/python -m unittest discover -s tools/contracts/tests -v
.cache/contracts-venv/bin/python -m tools.contracts export --output artifacts/service-descriptors.json
```

The schema suite checks message fixtures and request/reply exchanges, including generated-descriptor consistency, byte/range/hash limits, ambiguous lookup, missing receipts, unsafe retry advice and correlation errors. The Python validator adds no guest dependency. The [conformance scenarios](architecture/service-contract-cases.md) assign the wider stateful fault tests to #43 and their real guest owners; their inventory alone is not a passing backend test.

The read increment adds `python -m tools.contracts read-check` for a bounded immutable host backend and `read-native --evidence TERMINAL_JSON` for the shared range exchanges from native terminal evidence. Both explicitly cover one operation and the `complete_bounded_ranges` fixture profile. The general contract permits shorter non-EOF progress; this profile requires the complete requested range up to EOF. See [native read reproduction and limits](FILES-READ.md). It is not the full #43 stateful backend or native/function/MCP adapter comparison. Record actual results and backend identity for each run; generated descriptors and host fixtures cannot substitute for guest authority or disk execution.

The completed-operation increment adds `operations-check` for a bounded host backend and `operations-native --evidence RECOVERY_JSON` for 30 shared exchanges across ten native replacement vectors and both lookup forms. The full native recovery mission also verifies lost replies, authority, historical lookup and five publication fault boundaries. The profile uses two global retained records and a volume-wide epoch, shared with legacy receipts; it does not offer independent workspace retention, queued/running states or cancellation. See [workspace operation reproduction and limits](FILE-OPERATIONS.md).
