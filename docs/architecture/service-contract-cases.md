<!-- SPDX-License-Identifier: Apache-2.0 -->

# Service contract conformance scenarios

Reviewed expectations for [service v1](../SERVICE-CONTRACTS.md), 2026-09-10. These scenarios are **not executed stateful/guest tests**. The checked-in message fixtures validate their encodable forms only. #43 implements the deterministic backend and broken variants; #22/#39 repeat the applicable cases through real guest tools/MCP. [Authority cases](authority-cases.md) supply the underlying grants.

Fixture: trusted context C may read/replace A's precreated file, inspect/cancel its operations and read scoped events. It cannot access workspace B. Helper H may read A only. Owner O has a separate control path. The file begins at v1. All disk fixtures are disposable; no model or personal file is required.

| ID | Stimulus | Required observation / first real owner |
| --- | --- | --- |
| S01 | Discover, read v1, replace with epoch/key, inspect and verify v2/hash | Same file and receipt through native, tools and optional adapters, without vision; #12/#22 |
| S02 | C or H names B, raw block resource, another operation, or supplies a forged subject | Enforcing service denies; no unrelated state/receipt leak, server authority is not lent; #13 |
| S03 | Owner edits to v2 between read and commit | New-key replace at v1 fails without changing v2; deliberately admission-only backend fails this case; #12/#13 |
| S04 | Same key/arguments delivered twice, including a dropped first reply | One operation/effect; lookup by workspace/epoch/key recovers the ID; dedup precedes stale-version rejection after authorization; #12/#22 |
| S05 | Same key but different content, resource or expected version | Idempotency conflict, no second effect; #12/#22 |
| S06 | Lose a reply, then fill/compact retained records and restart | Epoch advanced durably before forgetting; old unknown key never admitted afresh, client reconciles; #12/#22 |
| S07 | Terminate VM after pending record, staging, commit record, flush or before reply | Recover old state/no committed receipt or new state/matching receipt; uncertain device/metadata recovery stays unknown and unavailable for blind retry; #12/#22 |
| S08 | Revoke C after admission but before commit, move its endpoint, then regrant | Old context and queued/helper work stay fenced; fresh grant does not revive them, pending earlier I/O is reported; #13 |
| S09 | Cancel queued work, race cancellation with commit, repeat cancellation | Cancelled only before effect; requested is not final, committed work remains succeeded/too_late; #12/#13/#22 |
| S10 | Client/service dies with queued and submitted work; new instance receives fresh grants | No execution under dead authority, original receipt/query is protected, incomplete work settles or stays unknown; #44/#12/#13 |
| S11 | Multi-range read while owner changes file | Subsequent pinned read conflicts; no mixed-version result or invented full-file hash; #12 |
| S12 | Overflow event ring, reconnect with old cursor, or change visibility | Explicit cursor expiration, establish head then re-observe/drain; buffered hidden events do not leak; #13/#22 |
| S13 | Exhaust live/receipt/data slots or lose storage before evidence reservation | Bounded no-effect rejection where established; control survivor progresses; unknown submitted writes are not labelled failed/none; #44/#12/#13 |
| S14 | Mismatched schema digest, unsupported version, malformed/truncated response, oversized/ambiguous bytes | Client rejects; pending mutation outcome remains unknown until authenticated reconciliation; #43/#22/#39 |
| S15 | Files/read permitted but model-provider export, credential extraction or capture absent | Read remains usable manually; external export/vision denied until independently granted; #13/#23/#40 |
| S16 | A future multi-step tool partially finishes before cancellation | Report each known effect; never represent the group as an atomic v1 replace or invent rollback; #43 then the owning future operation |

The golden mission exchanges specify expected values, not a fabricated successful run. Shape negatives include absent preconditions/receipts, invented authority arguments, unsafe retry advice and contradictory terminal states. Host tests additionally reject mismatched receipt content, uncorrelated replies, over-request reads and descriptor drift. Actual version ordering, durable epochs, revocation acknowledgments, interrupted I/O and owner progress require the stateful and guest implementations above.

S09's required observation is now partly checked against real guest evidence: [`activity-native`](../SERVICE-CONTRACTS.md) maps the native live-control facts onto the `operation` type and rejects a mapping in which an accepted stop becomes `cancelled`, a settling publication reports a rollback, an uncertain attempt claims a known effect, or a refused request returns a result instead of an error. The `operations.cancel` disposition, cancellation of queued work and the stateful backend remain #43/#12; the correspondence is a check over recorded evidence, not a guest implementation of the method.
