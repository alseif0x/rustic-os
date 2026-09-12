<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native capability discovery

A client should not have to guess what a running service implements. This first
#22 increment lets any bound native client ask the file service which logical
[service-v1](SERVICE-CONTRACTS.md) methods it implements right now, and the
bounds it actually enforces. The answer describes the service, never the caller:
availability is not permission and grants no right.

## Observable contract

| SDK call / shell command | Authority | Result |
| --- | --- | --- |
| `Client::capabilities` / `capabilities` | Any live grant; no particular right | One bounded report of the eight catalog methods and the enforced bounds |

The report lists every catalog method in its shared order, each `available`,
`degraded` or `unavailable`, plus `max_inline_bytes`, `max_page_items` and
`receipt_capacity`. It contains no object identity, file content, receipt or
authority, so a client learns what exists to attempt, not what it may do.

Availability is derived from the volume this service has actually mounted:

| Method | Reported by this service | Why |
| --- | --- | --- |
| `files.read` | `available` | The bounded range profile is implemented |
| `files.replace`, `operations.get` | `available` only on a scoped (format-3) volume, else `unavailable` | Completed replacements and receipts need that format |
| `capabilities.list` | `degraded` | Only this service's own methods are answered, not a registry |
| `capabilities.describe` | `unavailable` | The guest carries no reviewed contract digest |
| `operations.cancel` | `unavailable` | The [native live stop](FILE-ACTIVITY.md) is a separate profile, not this method |
| `events.read`, `system.status` | `unavailable` | Owned by services that do not implement them yet |

Enabling operations on a mounted volume changes the report; a method is never
advertised because the build could support it. Methods owned by other services
are reported `unavailable` rather than hidden, so a missing entry is always a
protocol error rather than a silent omission.

## Wire and ownership

ABI version 1 adds opcode 58 (`CAPABILITIES`). The request carries no fields.
The response is one 64-byte packet: `id=0`, `version=1`, `arg` packs
`max_inline_bytes` (low 16 bits), `max_page_items` and `receipt_capacity`, and
`count=8` with one availability byte per method in catalog order. Unknown
availability values, extra payload bytes and out-of-range bounds are rejected.
Old peers reject the opcode explicitly.

[The shared method identity](../crates/abi/src/services.rs) belongs to the ABI
crate, separate from [the wire codec](../crates/abi/src/files/capabilities.rs)
and from [the service's answer](../crates/file-service/src/capabilities.rs),
which reads the mounted volume and owns no other state. The shell command and
SDK call add no policy. No kernel change, dependency or `unsafe` boundary is
introduced.

## Manual and deterministic clients

The same question is answered identically for a person at the shell and for a
deterministic client, under different authority. `act PID capabilities` asks
through an owner-stepped native session whose helper holds read-only rights on
one file; its packed answer and bounds must equal the shell's, and the same
client's staged write is still refused. Learning that a method exists never
grants it.

## Correspondence and limits

`python -m tools.contracts capabilities-check` validates the declared shape and
`capabilities-native --evidence TERMINAL_JSON` validates a real run: every
reported entry must be a valid reviewed `capability`, the catalog identity and
order must be intact, the bounds must match the enforced profile, the claim for
`files.replace`/`operations.get` must match the volume support the same shell
observed, and the manual and deterministic answers must have been compared. The
checker validates the complete native read exchanges and image identity instead
of treating a nonempty evidence object as proof. Operation format support is a
separate unused-key probe, not proof that a replacement executed; scoped
replacement execution is covered by the recovery mission. Unexpected probe
errors fail validation rather than being interpreted as support.

This is not an implementation of `capabilities.list`: that output also requires
a service instance for the answering incarnation, which the guest does not yet
report for discovery, and `capabilities.describe` additionally requires the
reviewed contract digest, which the guest does not carry. Both remain
`specified_not_implemented`. Live registry aggregation across services, events
and the complete M1 mission stay in #22/#13/#15.

## Native acceptance

Run the checks in [DEVELOPMENT.md](DEVELOPMENT.md), including:

```sh
cargo test -p rustic-file-service --test capabilities --locked
python3 tools/terminal_test.py
.cache/contracts-venv/bin/python -m tools.contracts capabilities-check
.cache/contracts-venv/bin/python -m tools.contracts capabilities-native \
  --evidence artifacts/boot/terminal-test/terminal.json
```

The terminal suite queries discovery on a volume without scoped operations and
cross-checks the claim against a scoped `operation` lookup, then repeats it after reboot and
requires an identical answer. The recovery suite's saturated-execution group
queries the same call on a format-4 volume, where `files.replace` and
`operations.get` must be reported available. Host tests cover the volume
transition, a client with an unrelated right, revoked clients and malformed
requests and replies.

Adding an instance and reviewed digests is necessary but does not by itself
complete the logical discovery methods. Their pagination, visibility, method
profiles and schema-bound descriptors still need native implementation and
conformance. The bootstrap support vector above remains a separate API; see the
[continuation review](H2-CONTINUATION-REVIEW.md).
