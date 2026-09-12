<!-- SPDX-License-Identifier: Apache-2.0 -->

# Retained prevention causes (#47)

An explicitly migrated volume now retains why an admitted replacement was
prevented: an accepted cancellation, an expected-version conflict, or lost
execution authority. The cause and terminal sequence publish atomically through
the existing metadata writer. File data is not published by that transition.

This is the persistent foundation for the logical lifecycle. Public admission
status and [observation profile 1](FILE-OBSERVATION.md) still return the existing
coarse `Cancelled` state. [Observation profile 2](FILE-OBSERVATION.md) exposes the
retained cause through an explicitly selected native query. No service-v1
method, contract digest or MCP capability is newly advertised by this increment.

## Meaning and policy

`rustic_fs::AdmissionStatus::prevention` is `None` for `Admitted` and `Committed`.
For durable `Cancelled`, it contains one of these storage facts:

| Cause | Meaning |
| --- | --- |
| `Unknown` | The cause was not retained, including all format-4 prevention records. Migration cannot reconstruct the original intention. |
| `Requested` | An authorized cancellation prevented the effect. This does not promise that no other guard would have failed. |
| `VersionConflict` | Scheduled execution found that the saved expected file version no longer matched. |
| `AuthorityLost` | The service prevented execution after its saved authority failed: denial, revocation, expiry, detach, replacement generation or inaccessible scope. |

The service selects policy; the filesystem records it without interpreting grants.
The old explicit `EXECUTE` still returns a version error while leaving the record
admitted. Scheduled execution records `VersionConflict` instead of leaving a
failed ticket silently scheduled. An already accepted queued stop takes precedence
before execution is attempted. During active execution, a detected guard loss
takes precedence if both it and a stop precede prevention. This is a decisive
cause, not an ordered history of all requests or failures.

A repeated cancellation, prevention or admission retry returns the original
terminal record. It cannot relabel an old unknown cause or overwrite a more
specific one. Late cancellation cannot turn a committed record into prevention.
The terminal record does not retain a historical `cancel_requested` flag for a
successful late completion.

An I/O error or failed drain remains `Uncertain`; it must not create a no-effect
claim. Only explicit recovery can establish whether the durable record is still
admitted, prevented or committed. Neither remount nor lookup schedules work.

## Explicit format compatibility

The owner may run this sequence on a deliberately selected volume:

```text
enable-operations
enable-admissions
enable-prevention-reasons
```

The final command uses the existing private supervisor administration path.
There is no public client migration method. It requires admission support already
enabled and upgrades format 4 to format 5; mount and ordinary requests never
perform this upgrade. The filesystem API is `Volume::enable_prevention_reasons`.
After an uncertain migration, remount and explicitly retry it before treating the
upgrade as complete.

Format 5 uses recovery magic `RUSTREC4`. Admission byte 81 stores cause codes
0/1/2/3 in the table's order; bytes 82–511 remain zero. Only cancelled records may
contain a nonzero cause. Unknown codes, causes on other states, wrong magic and
nonzero padding are rejected even with correct checksums. Format 4 still requires
bytes 81–511 to be zero. Older receipts without admissions remain unchanged.

The upgrade completes both metadata banks before acknowledging success, so an
older format-4 reader cannot select a stale pre-upgrade fallback. It preserves
identities, arguments, receipts and unknown legacy causes. A completed retry
does no writes. Migration is one-way; downgrading requires separate tooling.

`prepare_prevention` requires format 5 and rejects format 4 with `Unsupported`.
The service explicitly selects its existing format-4 cancellation path on older
volumes, where the returned storage cause is `Unknown`. Ordinary cancellation
records `Requested` on format 5. No upgrade is inferred from a desired cause.

The volume still uses 174 sectors, two metadata banks and two global retained
records. Prevention still requires 14 write/flush commands. The upgrade requires
at most two metadata publications (28 commands). Guest quotas, publication
ownership, dependencies and unsafe boundaries are unchanged.

## Verification and limits

Run `cargo xtask check`, the contract/runner suites, `python3 tools/terminal_test.py`
and `python3 tools/boot.py run --mode block-user --timeout 45` using
[DEVELOPMENT.md](DEVELOPMENT.md).

Filesystem tests cut all 28 migration commands and all 14 cause-publication
commands, including tears around the new byte, and recover both flushed and
visible media. They require exact causes or the preceding admitted state,
preserved legacy receipts, no automatic writes on mount, immutable replay,
explicit version rejection and semantic validation of recomputed-checksum media.
These are host disk models, not physical power-loss guarantees.

Service tests compare format 4 and 5 under queued stops, conflicting edits and
lost authority. Every active publication command is tested with stop alone and
stop plus revocation. Late completion has no prevention cause; failed drain
remains uncertain and recovers the original admitted record.

The existing two-boot terminal mission uses real SDK/IPC calls and an independent
disk decoder. It migrates a legacy cancelled record without inventing its cause,
checks idempotent upgrade/replay, explicitly rotates terminal history, and retains
requested cancellation and scheduled version conflict across reboot. The
`block-user` terminal volume retains `AuthorityLost` after revocation during real
pending admission I/O; its late committed operation has no prevention cause.
Its separate pending volume remains format 4, preserving native compatibility
coverage. Disk hashes verify zero replay writes.

The first expanded native replay exposed a stack-guard page fault in the terminal
admission fixture's mount path. Separating initial provisioning into its own
non-inlined setup function removes its temporary workspaces from replay mounting.
Both VM phases must pass after that correction; process stack and event quotas
are unchanged. This verifies the reproduced path, not a formal worst-case stack
bound for every possible program.

These tests establish stored causes and service policy. Public observation
profile 2 adds cause visibility and manual/deterministic client parity. Logical
failed/cancelled mapping, actual `operations.cancel` and negotiated catalog support
remain the next integration work in #47.
