<!-- SPDX-License-Identifier: Apache-2.0 -->

# H2 continuation review — 2026-09-12

This review compares `3a78229` with `5ee7bb4`: seven commits including the
instruction file and an evidence-inventory fix. It reviews code, claims, native
fixtures and failure detection. It is not a whole-OS security audit.

This is a historical review of those revisions. Its proposed next scheduling slice
is now implemented in [bounded scheduled file execution](FILE-SCHEDULING.md),
which records current behavior, validation and remaining acceptance.

## Baseline and work worth retaining

At `3a78229`, another authorized client could inspect or request a stop during
explicit execution, while the initiating call still waited for settlement.
The subsequent changes add a native staging/queue-pressure scenario, an unread
stop-reply scenario, a host correspondence checker, a file-service support
vector, and manual/deterministic discovery comparison. They preserve the existing
publication owner and narrow service/SDK boundaries; they do not add kernel
implementation, dependencies, disk formats or new unsafe code.

[CI 34692806010](https://github.com/alseif0x/rustic-os/actions/runs/34692806010)
passed all four jobs at `5ee7bb4`. Its direct run verified 22 boot scenarios,
31 recovery groups/62 VM boots and 1,025 first-phase terminal commands across
the two-boot terminal mission. Command counts include polling and may vary by run.
This is real progress, but a green suite only establishes the properties it checks.

## Reproduced findings and corrections

| Finding | Consequence | Correction |
| --- | --- | --- |
| Activity correspondence counted eight entries without checking unique required names or each case's verification flag. | A duplicated early-stop case could replace lost-stop coverage and still pass. | Require the exact unique inventory and verified cases. |
| Native flags were coerced with `bool`, terminal IDs accepted Python booleans, and running-with-stop was accepted despite the ABI forbidding it. | Invalid evidence could invent cancellation or pass an impossible native state. | Validate native identity, bounds, exact flag/integer types and forward-only phases before mapping. |
| Committed activity cases skipped operation-schema validation because no case-local receipt was captured. Refusal mappings were checked only for field names. | Success lacked its own validated receipt; expired/unavailable advice could violate service-v1. | Capture the matching completion through native IPC, validate its identity and receipt, and check every refusal against the real exchange contract. |
| Discovery accepted any 64-character image identity and treated a nonempty `read_contract` object as successful evidence. | Even a failed read report could support an advertised read capability. | Reuse the complete native read validator, including image identity and all range exchanges. |
| The operation-support probe treated every failure except Unsupported as support. | Busy, denial or protocol failure could be mistaken for an implemented mounted format. | Accept only the explicit unused-key outcomes; reject unrelated or ambiguous errors. |
| The typed capability decoder did not check its opcode. | A differently tagged packet could decode as a capability report when used directly, although the SDK transport already checks its expected reply opcode. | Enforce the tag in the owning ABI decoder and add a negative test. |

Six synthetic review probes reproduced acceptance of invalid evidence while the
original 49 contract tests passed. The corrective tests cover those failures and
the additional receipt, transition, refusal and opcode boundaries. The corrected
increment passed local `cargo xtask check`, 54 contract tests, 152 runner tests,
31 recovery groups/62 VM boots and the two-boot terminal mission with deterministic
discovery parity. Native read, completed-operation, activity and discovery
validators all passed against that newly collected evidence. All six invalid
review probes are now rejected. Host probes do not themselves prove native
execution; publication CI and remaining acceptance are tracked in #47/#22.

## Correct interpretation of the remaining work

**Staging pressure is not a background execution queue.** The saturation case
meaningfully checks retained staging, an undrained peer and control progress.
It does not complete scheduled admission or full queue/backpressure behavior.
The initiating EXECUTE still waits. Retained Admitted means prepared, not a
promise that the service has scheduled execution; the correspondence checker now
refuses that conversion.

**Four client bindings are already reachable.** [Mount](../apps/supervisor/src/work/mount.rs)
binds the shell at slot 0 and supervisor at slot 1;
[utility launch](../apps/supervisor/src/work/launch.rs) assigns slots 2 and 3.
The two-utility policy does not imply only two occupied file-service slots.
Occupancy, a full staging table, undrained transport and a full execution queue
are different conditions. Test the relevant combinations with explicit control
actors before proposing a product quota increase.

**Publication ownership is engineering work, not an external blocker.** The
exclusive volume/disk borrow correctly protects storage. Design bounded
scheduling state and reply ownership around that boundary, with one active
publication and capacity derived from all retained records. Do not bypass it
with raw pointers or make the dispatcher own unrelated subsystems. A design
spike should prove admission acknowledgement, active work, fresh authority and
owner progress before expanding the implementation.

**Discovery is a useful bootstrap API, not completed M1.** Current instance and
contract digests are necessary next pieces, but session visibility, pagination,
method-specific profiles and native conformance remain too. A local vector
listing unsupported methods is distinct from the reviewed logical registry.
Keep that distinction explicit instead of advertising complete capabilities.list
or capabilities.describe after adding two fields.

## Continuation

1. Accept the corrected evidence/decoder increment against host and native checks.
2. Finish one bounded #47 scheduling design and native acceptance slice. Preserve
   the useful saturation and lost-reply cases; extend them for scheduled work.
3. Integrate versioned #22 discovery and the complete M1 flow over the same SDK
   used by manual clients. Keep #13/#15/#43 obligations separate and traceable.
4. Retain one authoritative issue status with evidence and remaining criteria.
   A local handoff is useful but does not update GitHub acceptance by itself.

The implementation should be retained and reviewed incrementally. The findings
justify stronger adversarial validation and explicit architectural acceptance,
not discarding the native work or changing the OS vision.
