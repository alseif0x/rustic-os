<!-- SPDX-License-Identifier: Apache-2.0 -->

# Scheduled execution: response loss, restart and device faults

This #47 increment exercises the [native scheduling API](FILE-SCHEDULING.md)
through real IPC and VirtIO commands. It adds failure coverage around the existing
service implementation, plus a bounded deterministic client fixture for an unread
terminal status response. It does not change the scheduling/storage algorithm,
the kernel, authority grants, disk format or the service-v1 support advertised by
discovery.

## Reproducible cuts

Every case starts with two durable admissions for the same original file version:
the first contains `first`, the second `second`. Only one publication owns storage.
Both tickets exist while the selected real device completion is held. The pending
ticket cannot bypass the expected-version guard when it later reaches execution.

| Case | Interruption | Recovered first / second record | Final file |
| --- | --- | --- | --- |
| `late_header` | CANCEL-only stop while header completion is held (15) | Committed / Cancelled | `first` |
| `late_flush` | CANCEL-only stop while final flush is held (16) | Committed / Cancelled | `first` |
| `lost_stop` | Cancellation acknowledgement is never read during the first write | Cancelled / Committed | `second` |
| `lost_result` | Terminal status reply is ready but never read | Committed / Cancelled | `first` |
| `restart_data` | Restart the file-service process during the first write | Admitted / Admitted | `before` |
| `restart_flush` | Restart the file-service process during final flush | Committed / Admitted | `first` |
| `io_data` | EIO on the first write, after an accepted live stop | Admitted / Admitted after explicit recovery | `before` |
| `io_flush` | EIO on final flush, after an accepted late stop | Committed / Admitted after explicit recovery | `first` |
| `pressure` | Full retention/staging, unread client replies, active and pending stops | Cancelled / Cancelled | `before` |

Evidence names use the `scheduled_failure_` prefix. Numbers 15/16 identify the
existing publication command sequence, not elapsed time. The fault configuration
injects a real QEMU blkdebug EIO; a held completion remains owned by the actual
VirtIO driver. These are selected reproducible cuts, not physical power-loss
guarantees or exhaustive crash testing.

## What the results establish

A late stop acknowledges `settling` with cancellation requested. It cannot promise
rollback. When that first publication commits, the second admission's saved file
version is stale; its payload must not replace the completed first publication.
Conversely, an early accepted stop can prevent the first publication while the
second progresses normally, even if the cancellation client never reads its reply.

Scheduling has no unsolicited terminal response. The result-loss case therefore
discards the response to `admission_get` after settlement. The deterministic actor
uses an INSPECT-only binding, sends one exact GET and waits at most 100 ticks for
reply readiness. It does not receive/decode that reply. Readiness is not success:
the owner separately obtains the durable record and its matching receipt. A later
typed call on the actor's binding rejects the stale correlation instead of treating
the old reply as a new result. Read-only recovery leaves the volume bytes unchanged.

The existing owner restart job replaces the actual file-service process while a
device command is pending. The job reports its draining phase; owner commands
remain responsive. Once the fresh file binding is available, the native client
queries both records. Pending work is still admitted and has no live queue ticket.
Old/new process identities, reclaimed resources and a further VM reboot establish
that this is service restart followed by durable recovery, not a host simulation.

For EIO, the public query must first report `Uncertain`. The service abandons the
volatile pending ticket. Only explicit remount/recovery establishes the states in
the table. A committed record after the failed final flush does not retroactively
make the earlier uncertainty a successful response. The evidence validator keeps
that attempt in `reconciling` with effect `unknown` before using recovered facts.

The pressure case simultaneously occupies both retained records, both staging
slots and all four client bindings, while one utility leaves its replies unread.
The other utility can still schedule pending work; the owner can inspect and stop
both tickets. After prevention settles, the stalled client can drain its replies.
Explicit revocation reports one retained staging buffer discarded per utility,
proving publication did not consume those buffers. Cleanup returns native resource
counts to their original values.

## Evidence and validation

Each case includes a second VM boot, exact retained identities, the resulting file
version and hash, and an independent reader of the selected on-disk metadata and
file bytes. Every committed admission must have its own native completion receipt
matching its identity, originating instance, previous/resulting version, size and
payload digest. Both retained records and the unrelated file are checked. Read-only
reboot/query must leave the complete selected volume prefix unchanged.

The [native fixtures](../tools/terminal_support/scheduled_failures/__init__.py) split
late cancellation, response loss, process restart, device faults and pressure into
separate modules. The [evidence checker](../tools/contracts/scheduled_failure_evidence.py)
runs as part of `operations-native`. Negative tests reject missing/duplicate cuts,
false rollback, wrong phases/flags, unrelated receipts, unproven reply readiness,
unchanged service identity, loss of uncertainty, replay of pending work, incorrect
file versions and incomplete resource/pressure evidence. Synthetic test fixtures
remain separate from collected native evidence.

Run the configured procedures in [DEVELOPMENT.md](DEVELOPMENT.md):

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
.cache/contracts-venv/bin/python -m unittest discover -s tools/contracts/tests -v
python3 tools/boot.py run --mode recovery-test --timeout 60
.cache/contracts-venv/bin/python -m tools.contracts operations-native \
  --evidence artifacts/boot/recovery-test/recovery.json
.cache/contracts-venv/bin/python -m tools.contracts activity-native \
  --evidence artifacts/boot/recovery-test/recovery.json
```

The combined inventory is 44 recovery groups across 88 VM boots. Its common gate
requires 27 held-I/O observations, 62 activity replies and 17 expected uncertainty
responses. The nine new groups supplement the four original scheduling groups and
the prior explicit-execution regression. Host tests for this increment include 64
contract tests and 155 runner tests. Local `cargo xtask check`, both test suites,
all 88 recovery boots and the ordinary terminal (920 first-phase commands across
two boots) passed. Native read, operation, activity and discovery validators passed
against build `dedcc36bb62834c5`, kernel SHA-256
`4f3bc9c5c20a1119c04b21f0d4c005990a3e3438d6d3f5e7f8f036b74cf92fe1`.
Command counts vary with bounded polling. Publication CI belongs to the delivery
record in #47.

The expanded reports require the [executor's 128 KiB recovery summary budget](EXECUTOR.md#native-terminal-acceptance).
The former 64 KiB limit rejected collection after all 88 isolated boots had passed.
Normal export and failure capture now share the fixed bound; five additional host
tests cover accepted boundaries, oversized rejection and cleanup, bringing the runner
suite to 160 tests. Reports from other modes retain their existing limits.

## Remaining acceptance

These cuts establish bounded native behavior; they do not implement the logical
`operations.cancel` method, unify admission/completion IDs or add structured retained
failure causes. Native evidence for a separate human edit before scheduling and the
complete scheduled denial matrix still needs to supplement the existing host tests
and explicit-execution denial cases. Those requirements must not be checked off
solely because the shared controller already has another fixture. Method/profile
discovery and the complete deterministic mission remain #22/#15 integration work.
