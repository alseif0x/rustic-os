<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native lifecycle profile selection

#22/#47 clients can select one implemented [service-v2 lifecycle](FILE-LIFECYCLE.md)
method on their current file-service connection before using it. This is a bounded
native binding. The original [opcode-58 support vector](DISCOVERY.md) and all v1
schemas retain their meanings, including unavailable **v1** `operations.cancel`.

## Selection and use

```rust
use rustic_sdk::abi::services::Method;

let mut selected = files.negotiate_lifecycle(Method::OperationsGet)?;
let descriptor = selected.descriptor();
let current_responder = selected.responder();
let operation = selected.inspect(admission_id)?;
```

Choose `Method::OperationsCancel` and call `selected.cancel(admission_id)` for
cancellation. Selection checks the exact expanded contract digest, method,
version, profile and bounded reply shape. Selecting inspection cannot authorize
the cancellation method. The borrowed `LifecycleBinding` has private fields and
keeps exclusive access to its client: the caller cannot rebind that client or
transfer the selection to another client while it remains in use.

The response reports `available` on a ready admission-capable volume (formats 4
and 5), or `unavailable` on older supported volumes. It does not migrate storage.
The SDK permits examining an unavailable descriptor but refuses its operations
with `Unavailable`. A poisoned volume returns its actual error instead of healthy
support. Selection during an active publication returns `Busy` through the
existing bounded dispatch path: select before starting work or explicitly try
selection later. Already selected clients can use the existing live lifecycle
inspection/cancellation paths during publication.

Every selection requires a live grant. Its answer describes implementation support,
not the caller's rights or visible objects. Every subsequent operation independently
checks the current peer, context, scope, subject, rights and expiry. Revocation
after selection still takes effect. CANCEL does not imply INSPECT. No selection
reserves capacity or promises that a later request will succeed.

## Current connection versus historical origin

`responder()` is the sender authenticated by the kernel IPC envelope, pinned by
the SDK's correlated RPC transport. `context()` identifies this client's grant
generation on that connection. Neither is the operation's `service_instance`:
that field remains the historical origin of a durable operation across restarts.
Read-only selection never allocates a durable origin or increments a disk sequence.

The responder is meaningful within the current boot and connection only. Process
numbers can recur after reboot; they are not global identities, cryptographic
nonces or persistent registry IDs. A descriptor copied out of the binding is a
snapshot and cannot reconstruct an executable selection. Closed or poisoned RPC
connections fail normally. Rebinding requires a fresh selection; the supervisor
retires old service clients when it restarts the file service.

This avoids introducing a clock-derived identity or writing storage merely for
discovery. A future multi-service registry must define trusted issuance, session
visibility, aggregation and restart semantics before claiming a persistent or
network-wide service identity.

## Native profile and wire

The current selector is `(method, contract version=2, native profile=1)`.
Profile 1 is named `stable_admission_minimal_cancel`; it binds `operations.get`
to observation profile 2 and its stable-ID projection, and `operations.cancel`
to the minimal opcode-61 acknowledgement. It does not reinterpret v1 methods.

Opcode 62 (`negotiation::DESCRIBE`) uses the existing 64-byte packet and one
correlated response per method. There is no multi-packet descriptor collector or
implicit registry snapshot. Queries for separate methods are separate observations.

| Field | Request | Successful reply |
| --- | --- | --- |
| `id` | Method ID: 5=get, 6=cancel | Same method |
| `version` | Contract version 2 | 2 |
| `arg` | Native profile 1 | 1 |
| `context` | Current grant generation | Same context |
| `count` | 0 | 36 |
| `data[0..4]` | Zero | Availability, retained operations, execution tickets, active publications |
| `data[4..36]` | Zero | Reviewed descriptor SHA-256 |
| `data[36..40]` | Zero | Zero |

Limits are **two retained operations, two total execution tickets (including
an active ticket), and one active publication**. These are enforced capacities,
not free slots; legacy receipts also occupy retention. Native request and reply
size is fixed at 64 bytes. Neither method accepts inline file content. Unknown
versions/profiles return `UnsupportedVersion`; unsupported methods return
`Unsupported`. There is no downgrade or automatic retry of a mutation.

Hashes use `Catalog.digest`: each catalog entry plus its fully expanded
input/output/request/response schemas. Guest and SDK carry the reviewed values
in [the ABI bundle](../crates/abi/src/files/negotiation/reviewed.rs). Contract
tests recompute them from the actual v2 schemas and reject drift. A digest
detects a contract mismatch; trusted IPC supplies peer identity. The digest
itself is not publisher authentication or a capability.

## Manual and deterministic acceptance

```text
lifecycle-profile operations.get
lifecycle-profile operations.cancel
inspect-negotiated ADMISSION_ID
cancel-negotiated ADMISSION_ID
act PID profile-get
act PID profile-cancel
act-admission PID inspect-negotiated ADMISSION_ID
act-admission PID cancel-negotiated ADMISSION_ID
```

Existing low-level commands remain available. Deterministic profile reports
compare limits and the authenticated responder with the manual client; success
also requires the SDK to accept the full exact digest. The terminal fixture
checks unavailable support before migration, available support on v4/v5,
independent INSPECT/CANCEL denials, actual negotiated v4 cancellation, legacy
Unknown preservation, revoked selection, restart retirement and unchanged
historical observations after restart/reboot. Independent disk snapshots check
the retained outcome and that read-only selection does not modify storage.

Run `cargo xtask check`, both Python test suites and `python3 tools/terminal_test.py`
as in [DEVELOPMENT.md](DEVELOPMENT.md), then:

```sh
.cache/contracts-venv/bin/python -m tools.contracts negotiation-native \
  --evidence artifacts/terminal-test/terminal.json --output artifacts/negotiation-native.json
```

The validator first checks the full lifecycle prerequisites, then verifies ten
native descriptors against the reviewed bundle, five typed inspections and two
cancellation acknowledgements. It consumes identified guest evidence; it does
not run a VM. CI also runs it on the direct terminal report. Boot inventories,
evidence-size budgets, storage format, dependencies and unsafe boundaries remain
unchanged. General registry descriptors, authorization-filtered tool visibility,
M1 integration and the remaining shared failure vectors remain separate
acceptance work in #22/#47/#43.
