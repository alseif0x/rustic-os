<!-- SPDX-License-Identifier: Apache-2.0 -->

# Issue #22: historical snapshot

Source: https://github.com/alseif0x/rustic-os/issues/22

Captured on 2026-09-11 before the owner-approved plan consolidation, at code baseline 79ae9091bb19dbe2a3b3c8d830ad00f36bdde548. The original body below is preserved verbatim. Historical next steps and capability statements are not the current plan. Consult the live issue and the current subsystem guides. Earlier failures and limitations remain evidence.

---

## Outcome
Operate real RusticOS services through semantic tools with the same authority as a conventional client.

**Plan:** #1 · **Milestone:** H2 · **Type:** Implementation
**Hard dependencies:** #6, #14, #15, #43.

## Deliverables
- [ ] Small catalog for state/capabilities, processes/services, and authorized files; descriptors consistent with schemas/manifests and versions.
- [ ] Validate parameters, identity, permissions, preconditions, and limits in services; discovery filtered by capability and session.
- [ ] Query/track by ID, cancellation, partial results, and subsequent verification; documentation for registering new tools with separate authority.
- [ ] Apply cases from harness #43 to the real guest backend and publish a coverage matrix of available capabilities.

## Acceptance tests
- [ ] A deterministic client queries state, changes an authorized file, and reads it again to verify, without images/OCR/clicks.
- [ ] The same case through a manual client and a tool produces equivalent results and permissions; out-of-scope writes/revocation are denied.
- [ ] Missing capabilities, invalid input, duplication, and partial failure do not generate false success; distinguish the real backend from fixtures.

## Scope and limitations
Does not include MCP, a model, or candidate builds. Avoid using a generic shell as a substitute for the entire catalog.

## Closure
Link the implementation or decision, configuration/versions, tests, and evidence; record the review and limitations in accordance with #1. Checkboxes remain pending until those results are available.

## Native stable references and version-pinned reads — 2026-09-10
[Implementation](https://github.com/alseif0x/rustic-os/commit/eb1be0f8605744e9bbb49e97160ac17b70a90cb9) and [contract, SDK, commands and proof boundaries](https://github.com/alseif0x/rustic-os/blob/eb1be0f8605744e9bbb49e97160ac17b70a90cb9/docs/FILES-READ.md) deliver the bounded native `files.read` binding. Workspace/resource identity uses persistent lineage and monotonic directory/object IDs; references survive service/VM restart, but never confer or restore authority. Deletion/recreation cannot reuse the old identity. Every fragment rechecks the authenticated peer/context, actual object scope, selected workspace ancestry and pinned version.

The shell's `cat`/`read-ref` and deterministic native C/H clients share the typed SDK. OPEN returns the observed version, size, SHA-256 of the requested returned bytes and current retry epoch; the SDK collects up to 1 KiB over unchanged 64-byte IPC, verifies the final hash and clears caller output on failure. There is no retained read lease, extra resident process or quota increase. A read-only helper observes an epoch without acquiring receipt-inspection authority. Legacy lineageless volumes retain manual `cat` compatibility, while the new API reports Unavailable until explicit owner upgrade.

Validation: 109 Rust tests with configured formatting/Clippy/native builds, 91 runner tests and 29 contract tests; all 22 direct VM scenarios passed locally, including terminal acceptance across two boots and recovery across 18 boots. Shared host/native conformance covers 15 range cases; host challenges also reject ignored versions and foreign resources. Native C/H assertions exercise full reads, edit and revocation between chunks, live-client retirement at service restart, fresh issuance, workspace recreation and reclamation. The harness rejects corrupted/incomplete/ambiguous responses. [Publication CI](https://github.com/alseif0x/rustic-os/actions/runs/34529363684) and [resource evidence](https://github.com/alseif0x/rustic-os/blob/eb1be0f8605744e9bbb49e97160ac17b70a90cb9/docs/MEASUREMENTS.md#native-read-calibration--2026-09-10) retain the read increment's evidence. That CI run passed check, measurements and sandbox but failed direct recovery with an unexpected post-rotation `Uncertain`; its read-native checker step was consequently skipped. The isolated recovery sequence passed independently. #46 preserves that unexplained result and the follow-up deadline investigation; later successes do not erase either failed CI run. The exact publication tree `5a672a951d35010778a6d8546fdc246422dfe614` also passed isolated terminal and recovery acceptance using snapshot `0ae3cb86046331f02d1ee94b09c6e24274ae8702`: jobs `f17faaf608a647f2bd37ddc55e6d80b1` and `dc5ac7549f3e434ab4034ccc19ebb3e5`, with no cleanup errors. The terminal completed 960 first-boot commands and its second persistence boot; polling counts vary.

Coverage is one operation and the `complete_bounded_ranges` fixture profile. General v1 may return shorter non-EOF progress; this fixture deliberately requires the requested range up to EOF. The eight-operation catalog remains a specification, not live discovery or a full native/function/MCP comparison. Original modules retain separate ABI, filesystem facts, service policy, SDK validation and host-evidence responsibilities. SHA-256 uses pinned RustCrypto software code with complete preserved MIT notices; the kernel has no hashing/SDK dependency. Implementer plus additional agent reviews and automated checks; no independent security audit.

**Partial milestone handoff:** the manual client and deterministic native client now share the implemented read operation with the same service-side authority. A UART conformance adapter is a test boundary, not a generic shell tool offered as the completed catalog. Live capability discovery, the full semantic tool catalog and complete mutation/operation mission remain pending, so existing whole-mission checkboxes stay open.

**Next dependency:** complete cancellation/settlement semantics and the remaining discovery/status/event services, with shared conformance in #43, before advertising the complete deterministic tool catalog. The native read → replace → recover a lost reply → verify bytes slice is now implemented below. UART remains an acceptance adapter, not a generic shell substitute for the product tools. MCP integration remains later work.

## Workspace replacements and completed operations — 2026-09-11
[Implementation](https://github.com/alseif0x/rustic-os/commit/4bf5d03b9fe1090e2f79ee42ddfe551f4f3adb83) and [contract, SDK and reproduction guide](https://github.com/alseif0x/rustic-os/blob/4bf5d03b9fe1090e2f79ee42ddfe551f4f3adb83/docs/FILE-OPERATIONS.md) implement the synchronous completed_operations profile of files.replace and operations.get. Retry identity is trusted subject + volume lineage + monotonic workspace root + epoch/key. Lookup accepts the operation ID or workspace/epoch/key, preserves the original instance and SHA-256 receipt after edits/deletion/restart, and never grants a fresh write. Each fragment checks current authority; hidden and absent operation IDs are indistinguishable.

Explicit owner migration publishes format 3 in both metadata banks, preserves legacy records without inventing workspace identities, and retains the atomic data/version/receipt transition. Staging remains volatile. Two global retained slots and a volume-wide epoch remain shared with legacy receipts; there is no independent workspace retention or queued/running/cancellation claim.

Validation: 130 Rust tests with configured formatting, Clippy and native builds; 140 runner tests; 33 contract tests. The complete local direct suite passes 22 scenarios, including 15 recovery groups across 30 VM boots and a real discarded IPC response followed by version-pinned read/disk verification. Ten binary/boundary vectors produce 30 shared replacement/lookup exchanges. Both delayed-device controls pass on the same recovery ELF. Exact publication tree d4cbe626985ce41e3a989eaee854378697e6be79 also passes isolated recovery and native exchange validation in job 5186f4e8f0a849a08c497a347bdd5c34 (snapshot f88bbea90f2a89b0e15f4c5cc795d3c521f064d1), with no cleanup errors. [Publication CI](https://github.com/alseif0x/rustic-os/actions/runs/34541997741) passes all four jobs on the first attempt: workspace checks, direct native/conformance and delayed-device acceptance, 36-boot measurement controls, and the complete 26-case isolated executor suite.

Review by the implementing agent and automated checks; no independent audit. No new unsafe boundary, kernel dependency or external dependency. Selected QEMU failures and torn-write models do not establish physical power-loss or offline rollback guarantees. Three logical methods now have bounded native profiles; the full eight-operation catalog, live discovery and native/function/MCP comparison remain open.
