<!-- SPDX-License-Identifier: Apache-2.0 -->

# Issue #43: historical snapshot

Source: https://github.com/alseif0x/rustic-os/issues/43

Captured on 2026-09-11 before the owner-approved plan consolidation, at code baseline 79ae9091bb19dbe2a3b3c8d830ad00f36bdde548. The original body below is preserved verbatim. Historical next steps and capability statements are not the current plan. Consult the live issue and the current subsystem guides. Earlier failures and limitations remain evidence.

---

## Outcome
Detect contract problems before all OS services are available.

**Plan:** #1 · **Milestone:** H2 · **Type:** Implementation
**Hard dependencies:** #4, #6.

## Deliverables
- [ ] Deterministic runner with a bounded test backend for state, files, and long-running operations; reuse schemas/versions from #6.
- [ ] Fixtures for authorization, version conflict, duplication, cancellation, partial error, and missing capability.
- [ ] Export cases and results reusable by native/MCP clients; identify the backend in each piece of evidence.

## Bounded comparative experiment
- [ ] Implement the initial set of eight operations from #6 with prebuilt fixtures and compare a native client, a function calling adapter with simulated calls, and a minimal host MCP adapter with an independent client. Does not depend on the final server #39 or pilot #23.
- [ ] Use the same data/backend/workload and record versions, p50/p95 latency, RAM, bytes per mission, calls, and integration cost. Separate transport from inference: simulation does not measure a real model's latency or quality.
- [ ] Add response loss followed by retry, a duplicate key with different arguments, revocation, cancellation with partial effects, and an expired cursor. Discard paths that alter authority or claim false success.
- [ ] Record runtime/portability requirements and budgets after the baseline, before selecting an implementation. Do not extrapolate host measurements to the guest; repeat conformance against RusticOS in #22/#39.

[Technical proposal and sources](https://github.com/alseif0x/rustic-os/blob/main/docs/architecture/agent-integration.md). This adapter prototype is limited and does not close #23/#39.

## Acceptance tests
- [ ] A deliberately incorrect implementation fails the corresponding test and does not produce a falsely completed mission.
- [ ] Reproducible suite without a model, credentials, or access to personal files.
- [ ] Results declare that the backend runs on the host; the same tests must pass against RusticOS in #22/#39.

## Scope and limitations
This harness accelerates design and testing; it does not demonstrate guest processes, permissions, or drivers. It does not block first boot.

## Closure
Link the implementation or decision, configuration/versions, tests, and evidence; record the review and limitations in accordance with #1. Checkboxes remain pending until those results are available.

## Traceability
Incorporated improvement: test semantics and failures early using the same specification the OS will use.

## Model-derived fault cases — architecture review 2026-09-10
The [finite operation model](https://github.com/alseif0x/rustic-os/blob/main/docs/architecture/operation-model.md) is an earlier design probe, not this harness and not implementation of #6.

- [ ] Reproduce a human edit and revoke/regrant between admission and effect; include deliberately broken admission-only and version-only backends to prove detection.
- [ ] Model an effect surviving without its receipt and document how the chosen backend reconciles it. A simulated atomic transaction cannot establish real disk consistency.
- [ ] Keep unknown external outcomes distinct from failed/not-started operations; a retry policy must not repeat an unqueryable, non-idempotent external effect blindly.

Carry these cases into guest acceptance when their real service/transport boundaries exist. The full adapter comparison and original acceptance criteria remain pending.

## Executable schema baseline — 2026-09-10
[#6's accepted contracts](https://github.com/alseif0x/rustic-os/blob/92f2b64d8cd9bda0c6d7e2c6cf8fe46b77841292/docs/SERVICE-CONTRACTS.md) now include nine modular schemas, an eight-method catalog, generated neutral descriptors, 62 message fixtures, nine exchanges and 14 host checks. Reuse them; do not create a second set of arguments or mistake those message-only checks for this stateful backend/adapter experiment.

Implement [S01–S16](https://github.com/alseif0x/rustic-os/blob/92f2b64d8cd9bda0c6d7e2c6cf8fe46b77841292/docs/architecture/service-contract-cases.md) where applicable, including deliberately broken commit/revocation ordering, a lost initial reply recovered by workspace/epoch/key, same-key/different-arguments conflicts and durable-epoch retention semantics simulated with explicit limitations. The first files.replace is atomic and at most 1 KiB: demonstrate partial task effects with a sequence of distinct operations, not a falsely successful partially replaced file. Unknown external effects require reconciliation. The native/function/MCP comparison remains pending, as do guest repeats in #22/#39; descriptor export alone does not establish provider compatibility.


## Native stable references and version-pinned reads — 2026-09-10
[Implementation](https://github.com/alseif0x/rustic-os/commit/eb1be0f8605744e9bbb49e97160ac17b70a90cb9) and [contract, SDK, commands and proof boundaries](https://github.com/alseif0x/rustic-os/blob/eb1be0f8605744e9bbb49e97160ac17b70a90cb9/docs/FILES-READ.md) deliver the bounded native `files.read` binding. Workspace/resource identity uses persistent lineage and monotonic directory/object IDs; references survive service/VM restart, but never confer or restore authority. Deletion/recreation cannot reuse the old identity. Every fragment rechecks the authenticated peer/context, actual object scope, selected workspace ancestry and pinned version.

The shell's `cat`/`read-ref` and deterministic native C/H clients share the typed SDK. OPEN returns the observed version, size, SHA-256 of the requested returned bytes and current retry epoch; the SDK collects up to 1 KiB over unchanged 64-byte IPC, verifies the final hash and clears caller output on failure. There is no retained read lease, extra resident process or quota increase. A read-only helper observes an epoch without acquiring receipt-inspection authority. Legacy lineageless volumes retain manual `cat` compatibility, while the new API reports Unavailable until explicit owner upgrade.

Validation: 109 Rust tests with configured formatting/Clippy/native builds, 91 runner tests and 29 contract tests; all 22 direct VM scenarios passed locally, including terminal acceptance across two boots and recovery across 18 boots. Shared host/native conformance covers 15 range cases; host challenges also reject ignored versions and foreign resources. Native C/H assertions exercise full reads, edit and revocation between chunks, live-client retirement at service restart, fresh issuance, workspace recreation and reclamation. The harness rejects corrupted/incomplete/ambiguous responses. [Publication CI](https://github.com/alseif0x/rustic-os/actions/runs/34529363684) and [resource evidence](https://github.com/alseif0x/rustic-os/blob/eb1be0f8605744e9bbb49e97160ac17b70a90cb9/docs/MEASUREMENTS.md#native-read-calibration--2026-09-10) retain the read increment's evidence. That CI run passed check, measurements and sandbox but failed direct recovery with an unexpected post-rotation `Uncertain`; its read-native checker step was consequently skipped. The isolated recovery sequence passed independently. #46 preserves that unexplained result and the follow-up deadline investigation; later successes do not erase either failed CI run. The exact publication tree `5a672a951d35010778a6d8546fdc246422dfe614` also passed isolated terminal and recovery acceptance using snapshot `0ae3cb86046331f02d1ee94b09c6e24274ae8702`: jobs `f17faaf608a647f2bd37ddc55e6d80b1` and `dc5ac7549f3e434ab4034ccc19ebb3e5`, with no cleanup errors. The terminal completed 960 first-boot commands and its second persistence boot; polling counts vary.

Coverage is one operation and the `complete_bounded_ranges` fixture profile. General v1 may return shorter non-EOF progress; this fixture deliberately requires the requested range up to EOF. The eight-operation catalog remains a specification, not live discovery or a full native/function/MCP comparison. Original modules retain separate ABI, filesystem facts, service policy, SDK validation and host-evidence responsibilities. SHA-256 uses pinned RustCrypto software code with complete preserved MIT notices; the kernel has no hashing/SDK dependency. Implementer plus additional agent reviews and automated checks; no independent security audit.

**Partial harness handoff:** `read-check` uses three immutable host fixtures and reports `guest_execution=false`; `read-native` checks real native UART exchanges and identifies the kernel hash. Both reuse the existing schemas and shared JSON vectors, without changing the catalog or descriptor bundle. The complete eight-operation stateful backend, response-loss/durable mutation/cancellation cases, provider adapter and independent MCP-client comparison remain pending. Do not close #43 from this read subset.

**Next conformance slice:** extend the completed-operation profile below with service-owned asynchronous admission, cancellation/revocation races and restart settlement. Retain hidden-record indistinguishability, malformed/ambiguous response rejection and original-receipt verification. Add discovery/event/status coverage and compare actual native/function/MCP adapters before closing the full eight-operation mission. Host fixtures and UART evidence remain separately identified.

## Workspace replacements and completed operations — 2026-09-11
[Implementation](https://github.com/alseif0x/rustic-os/commit/4bf5d03b9fe1090e2f79ee42ddfe551f4f3adb83) and [contract, SDK and reproduction guide](https://github.com/alseif0x/rustic-os/blob/4bf5d03b9fe1090e2f79ee42ddfe551f4f3adb83/docs/FILE-OPERATIONS.md) implement the synchronous completed_operations profile of files.replace and operations.get. Retry identity is trusted subject + volume lineage + monotonic workspace root + epoch/key. Lookup accepts the operation ID or workspace/epoch/key, preserves the original instance and SHA-256 receipt after edits/deletion/restart, and never grants a fresh write. Each fragment checks current authority; hidden and absent operation IDs are indistinguishable.

Explicit owner migration publishes format 3 in both metadata banks, preserves legacy records without inventing workspace identities, and retains the atomic data/version/receipt transition. Staging remains volatile. Two global retained slots and a volume-wide epoch remain shared with legacy receipts; there is no independent workspace retention or queued/running/cancellation claim.

Validation: 130 Rust tests with configured formatting, Clippy and native builds; 140 runner tests; 33 contract tests. The complete local direct suite passes 22 scenarios, including 15 recovery groups across 30 VM boots and a real discarded IPC response followed by version-pinned read/disk verification. Ten binary/boundary vectors produce 30 shared replacement/lookup exchanges. Both delayed-device controls pass on the same recovery ELF. Exact publication tree d4cbe626985ce41e3a989eaee854378697e6be79 also passes isolated recovery and native exchange validation in job 5186f4e8f0a849a08c497a347bdd5c34 (snapshot f88bbea90f2a89b0e15f4c5cc795d3c521f064d1), with no cleanup errors. [Publication CI](https://github.com/alseif0x/rustic-os/actions/runs/34541997741) passes all four jobs on the first attempt: workspace checks, direct native/conformance and delayed-device acceptance, 36-boot measurement controls, and the complete 26-case isolated executor suite.

Review by the implementing agent and automated checks; no independent audit. No new unsafe boundary, kernel dependency or external dependency. Selected QEMU failures and torn-write models do not establish physical power-loss or offline rollback guarantees. Three logical methods now have bounded native profiles; the full eight-operation catalog, live discovery and native/function/MCP comparison remain open.

## Publication mechanics proof, before the asynchronous profile — 2026-09-11
[746ba78](https://github.com/alseif0x/rustic-os/commit/746ba780cf016e24b115fbbde877b5149a9025a6) adds [host and native publication-boundary validation](https://github.com/alseif0x/rustic-os/blob/746ba780cf016e24b115fbbde877b5149a9025a6/docs/FILE-PUBLICATION.md), with executed evidence in #12. The native ring-3 fixture cancels at 16 settled pre-header boundaries, rejects late rollback, flushes the committed result and recovers/replays it after another VM boot. An independent disk reader checks original file/receipt content and unchanged replay bytes. Host tests separately challenge errors/torn sectors, legacy boundaries, drop/forget, adapter unwind and immutable historical replay.

This does not implement an IPC cancellation race or extend the completed_operations logical profile. Keep the eight-operation conformance checkboxes pending. The next profile must cover service-owned pending I/O, current cancel authority, durable admission/cancellation identity, revocation before publication, reply loss and restart/retry settlement. Volatile preparation cannot be a durable queued/running fixture, and missing cancellation evidence after restart cannot prove rollback.


## Owner revocation during pending logical publication — 2026-09-11
[Implementation](https://github.com/alseif0x/rustic-os/commit/7ceb76c0534f13a32216333ca52b20cee21f5472) and [ownership, settlement and reproduction](https://github.com/alseif0x/rustic-os/blob/7ceb76c0534f13a32216333ca52b20cee21f5472/docs/FILE-CONTROL.md) make the native completed-profile files.replace path poll one owned disk command while receiving private owner control. Volatile client authority/staging is owned separately from the borrowed volume. Every poll rechecks the current peer/context, rights and expiry; root revocation reaches helpers, while an unrelated root cannot cancel the operation. Revocation before header submission drains the admitted scratch command and preserves the previous file/version. Once the header may have been submitted, the writer must settle; it withholds the receipt under revoked authority, and fresh authorized lookup recovers the immutable committed result. A draining error remains Uncertain.

The administrator's revocation acknowledgement waits for settlement; unknown pending effects are never relabelled as rollback. Ordinary file requests remain queued, and new grants/storage administration return Busy during this logical commit. Reads, legacy mutations and storage administration still use synchronous I/O. This is not a general event loop, durable queued/running admission, an explicit cancel right or a public operations.cancel endpoint. Stable admission identities and retained cancellation records remain the next contract/storage increment, followed by public cancellation and restart/retry conformance. Keep this issue open.

#12 records the executed host/native/isolated results and publication CI. The native proof uses a kernel diagnostic that withholds completion observation after real submission; it is distinct from the existing QEMU EIO and physical power-loss scenarios. Three logical operations retain their bounded profiles; this does not add a fourth public method or complete the eight-operation catalog.


## Durable admission storage proof, separate from service conformance — 2026-09-11
[Implementation](https://github.com/alseif0x/rustic-os/commit/1fb4b7f751416705a7de947d4f6496914feffa9c) and [reproduction/limits](https://github.com/alseif0x/rustic-os/blob/1fb4b7f751416705a7de947d4f6496914feffa9c/docs/FILE-ADMISSION.md) add nine filesystem tests for admission/cancellation/effect/migration/retention cuts, torn sectors, semantic corruption, history, quota/version checks and abandoned writers. The real two-VM block-user fixture independently preserves admitted, cancelled and committed records. Its Python disk reader checks CRCs, identity/content/version agreement and unchanged replay hashes; isolated artifact export retains both bounded volume witnesses. #12 records the exact tree, isolated job, direct suite and CI.

These are storage facts. New fault cuts remain host models; the native fixture is not an asynchronous IPC exchange or operations.cancel conformance. Three existing logical methods continue to pass their bounded native profiles (15 ranges and 10 replacement vectors/30 exchanges). The next profile must combine pollable durable admission/terminal writes, current cancel authority, accepted/result framing, lost replies, restart and native I/O faults. Keep the eight-operation and adapter-comparison acceptance open.

## Pollable admission and authority evidence — 2026-09-11
[Implementation and proof boundary](https://github.com/alseif0x/rustic-os/blob/16a4e0fbfe0f8b9e7c4bcc423af0d89f64dfce5a/docs/FILE-ADMISSION-CONTROL.md) add host service challenges at all 14 admission, 17 execution and 14 terminal-cleanup pending positions, alongside metadata-guard cancellation/drop/forget/unwind checks. Native block-user now invokes the service library against actual copied block I/O and verifies fresh-authority inspection, prevented execution, durable cancellation and late committed settlement across two VMs. The independent oracle requires unchanged replay hashes; the evidence rejects missing service_control/fresh_authority assertions. Exact publication tree 90a9072174b59f412ecee3770099869416b63248 passes isolated job 89ab0467f86443d8bcf8931e7c856be7 with no cleanup errors.

#12 records the complete regression and [publication CI](https://github.com/alseif0x/rustic-os/actions/runs/34620525092). New callbacks are deterministic service fixtures; public asynchronous IPC, explicit cancel authority, response-loss/restart and selected native EIO conformance remain future work. The three existing logical methods retain 15 native read ranges and 10 replacement vectors/30 exchanges. No fourth catalog operation or full adapter comparison is claimed.


## Native explicit-admission evidence — 2026-09-11

[Implementation and native API contract](https://github.com/alseif0x/rustic-os/blob/79ae9091bb19dbe2a3b3c8d830ad00f36bdde548/docs/FILE-ADMISSION-API.md) expose durable preparation, coherent status, explicit execution and independent cancellation through authenticated file-server IPC, the Rust SDK and the terminal. Admission IDs (`ad_`) remain distinct from completed receipt IDs (`op_`). The client chooses when to execute; query, identical retry, service restart and VM reboot never resume retained work. CANCEL is a separate scoped right (bit 8), without READ/WRITE/INSPECT authority. Format 4 requires explicit owner activation after format 3.

Host tests challenge canonical identities/states, protocol separation, raw/typed SDK uncertainty, cancellation-only authority, cross-scope/subject denial and all 14 pending cancellation/revocation positions. The native recovery harness expands from 19 groups/38 boots to 23 groups/46 boots. Four new groups use actual SDK/IPC/file-service/VirtIO with an independently decoded disk: lost acceptance and pending reboot followed by explicit execution/cancellation; acceptance first-write EIO; cancellation first-write EIO; execution final-flush EIO. Repeat/status/reboot must leave selected volume hashes unchanged. Existing completed-profile conformance remains 10 vectors/30 exchanges and native read conformance 15 ranges.

These typed native admission packets do not claim conformance to the general service-v1 operations.cancel schema or add a fourth implemented logical method. Queue/running/cancel-requested states, public cancellation racing active execution, lifecycle events, discovery and adapter comparisons remain open. #12 records the exact revision, full local regression, isolated terminal job and publication CI status.
