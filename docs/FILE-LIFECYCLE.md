<!-- SPDX-License-Identifier: Apache-2.0 -->

# Retained operation lifecycle, service version 2

The subsequent [native negotiation binding](FILE-NEGOTIATION.md) selects these
methods by exact version/profile and reviewed descriptor digest on the current
IPC connection. The commands below remain the lower-level lifecycle entry points.

This bounded binding lets native clients inspect a prepared, scheduled or retained
file operation and request a stop under separate authority. It implements the next
logical boundary in [#47](https://github.com/alseif0x/rustic-os/issues/47).
It uses the existing two-record retention, one publication owner and explicit
admission/scheduling APIs. It is not a general task scheduler or an agent runtime.

## Version decision

The older [v1 specification](SERVICE-CONTRACTS.md) requires a historical
`cancel_requested` boolean on terminal results and includes the full operation in
the cancellation reply. Retained storage cannot establish that complete history,
and CANCEL must not implicitly authorize INSPECT. Its historical activity
correspondence also switches from an admission ID to a completion ID on success.

Those are breaking shape and authority changes, so this binding uses a separate
[v2 catalog](../contracts/services/v2/catalog.json) containing only `operations.get`
and `operations.cancel`. The eight v1 schemas, digests and native completed-receipt
binding keep their original meanings. This is a reviewed two-method subset, not
an implied upgrade of every method to v2. [Discovery](DISCOVERY.md) still reports
the existing v1 inventory. The separate [v2 selector](FILE-NEGOTIATION.md) now
checks these two reviewed method profiles on the current native connection.
General registry discovery and the complete deterministic mission remain
#22/#47 work; MCP remains #39.

The SDK exchanges binary packets, not JSON. The adjacent schemas define the
logical input/output and envelope; offline descriptors and evidence validation
check that boundary. They do not instantiate a JSON, HTTP or MCP server. Native
profile support is explicitly selected and never silently downgraded.

## One stable identity and honest retained facts

`operations.get` takes only `operation_id`, an `ad_...` admission reference. The
result keeps that same ID and the originating `service_instance`, including after
completion or reboot while retained. A successful result links its distinct
immutable `op_...` `completion_id`. Receipt inspection remains separately
authorized through the existing receipt API; this observation does not assemble
a receipt through multiple racing queries. Recovery of a lost admission response
still uses the explicit workspace/retry lookup before this binding is called.

| State | Effect | Additional result fields / evidence |
| --- | --- | --- |
| `prepared` | `none` | Retained Admitted with no volatile ticket; it will not execute by itself |
| `queued` | `none` | `stop_pending`; a bounded execution or prevention ticket exists |
| `running` | `none` | `stop_pending`; work has not reached uncertain publication settlement |
| `reconciling` | `unknown` | `stop_pending`; publication/settlement can still prove a committed result |
| `succeeded` | `committed` | `completion_id`, same lineage and later sequence than the admission |
| `cancelled` | `none` | Persisted Requested prevention |
| `failed` | `none` | `failure=version_conflict` or `access_denied`, from persisted VersionConflict or AuthorityLost |
| `prevented` | `none` | Persisted prevention with Unknown cause; no claim of a requested cancellation |

`stop_pending` describes only the current service stop latch. It does not identify
who initiated the stop; service prevention can set that latch too. Terminal states
have neither `stop_pending` nor historical `cancel_requested`. A decisive retained
cause is not a complete history of concurrent intentions. Legacy format-4
prevention therefore projects to `prevented`, even after explicit format-5
migration. Reads never migrate storage. I/O uncertainty or lookup denial never
becomes a fabricated terminal failure or rollback.

## Independent cancellation and its linearization point

`operations.cancel` takes the same admission ID and returns exactly
`operation_id` and `disposition`:

| Disposition | Meaning at the service's serialized decision |
| --- | --- |
| `requested` | A volatile stop was accepted for the matching prepared/queued/active record |
| `already_requested` | The same live ticket or controller already has its stop latch set |
| `too_late` | The record is already terminal; this does not reveal which terminal state or whether it committed |

The service checks current CANCEL, peer/context, subject, object scope and expiry
before deciding. The client does not inspect first and guess the disposition.
Replies carry no service instance, state, cause, stop history or receipt. INSPECT,
READ or WRITE alone cannot cancel, and CANCEL alone cannot inspect. Queued replies
are fenced again before delivery after revocation/expiry.

For prepared work the service inserts a stop-only ticket in the same bounded
queue. Its existing execution owner processes `stop` before any attempt to write
the admitted file, and persists prevention as authorized housekeeping. It does
not borrow another client's WRITE grant. Later revocation of the canceller does
not undo an already accepted stop; new requests are checked again. Scheduling a
ticket that already has a stop cannot clear it or rebind its original owner.

Acceptance is volatile until prevention settles. Restart discards all tickets,
including stop-only tickets, and leaves retained Admitted work prepared. Failed
drain/unknown settlement abandons volatile work and requires explicit recovery.
A stop during final publication can still produce `succeeded`; `reconciling`
must keep effect `unknown` until a retained fact is available. Lost, malformed or
misbound mutation replies become SDK `Uncertain` and are never automatically
retried. An acknowledgement alone never proves durable cancellation.

The scheduled transport can control either retained record during pending I/O.
Legacy blocking EXECUTE exposes only its own active record through the restricted
controller while it exclusively owns storage. Unavailable/hidden records yield
no guessed outcome. The contract does not promise durable stop delivery, fair
general scheduling, unlimited retention, event subscriptions or full cancel history.

## Native wire and modular ownership

Inspection reuses exactly one [observation profile 2](FILE-OBSERVATION.md) exchange
(opcode 60, explicit argument 2). `rustic_abi::files::lifecycle::Operation` validates
and converts that typed result; it has no filesystem or SDK dependency.

Cancellation uses opcode 61 within the existing 64-byte file packet version 1.
The request has `id=0`, `arg=2`, `count=16`, `version=admission.number`, and the
16-byte lineage followed by zero padding. The successful reply has `id=2`,
`arg=1/2/3` for requested/already_requested/too_late, the same admission number,
lineage and context, and no other payload. Status is zero on success. Error
packets remain correlated with all result fields zero. Unsupported profile
arguments return `UnsupportedVersion`; codecs reject mixed profiles, unknown
dispositions, nonzero padding and invalid identities.

ABI types/codecs, SDK transport, service cancellation policy, shell rendering,
diagnostic client and host evidence each live in separate modules. The queue
retains ownership of tickets; the publication controller retains its exclusive
volume/disk borrow. No kernel, unsafe boundary, dependency, disk format, retained
slot count, process quota or stack-size increase is introduced.

## Use and validation

```text
inspect-operation ADMISSION_ID
request-operation-cancel ADMISSION_ID
act-admission PID inspect|cancel|lost-cancel ADMISSION_ID
```

Rust callers use `Client::operation_inspect(id)` and `Client::operation_cancel(id)`.
The diagnostic actor reports lifecycle states prepared/queued/running/reconciling/
succeeded/cancelled/failed/prevented as values 1..8. `other` carries only the live
stop bit or completion sequence; `version` carries failure 1=version_conflict or
2=access_denied, otherwise zero. Cancellation reports disposition 1..3 with all
other diagnostic fields zero. These owner test commands grant no extra authority.

```sh
cargo xtask check
.cache/contracts-venv/bin/python -m tools.contracts lifecycle-check
.cache/contracts-venv/bin/python -m tools.contracts lifecycle-export
.cache/contracts-venv/bin/python -m unittest discover -s tools/contracts/tests -v
python3 -m unittest discover -s tools/tests -v
python3 tools/terminal_test.py
.cache/contracts-venv/bin/python -m tools.contracts lifecycle-native \
  --evidence artifacts/terminal-test/terminal.json --output artifacts/lifecycle-native.json
python3 tools/boot.py run --mode recovery-test --timeout 60
```

Host tests challenge shape/identity/history, corruption, no retry, separate rights,
prepared stop/restart, all 17 pending publication positions, late completion and
failed drain with both active and prepared stop tickets. Host evidence alone is
not guest execution. The actual terminal mission exercises two independently
bound native clients, prepared and queued prevention, running/reconciling state,
repeated and late stops, a discarded late-stop reply, independent disk facts,
empty authority denials, resource reclamation and retained observations across
reboot. Legacy Unknown and both structured failures use the existing prevention
mission. Its report includes 22 typed inspections and ten minimal acknowledgements.

The standalone native validator consumes those results and the identified kernel;
it does not run a VM. The full recovery regression retains 49 groups / 98 boots,
and direct/isolated inventories remain 22/26 scenarios. The lifecycle delivery passed
80 contract and 174 runner tests. This increment adds terminal workload inside
the existing two-boot scenario, with unchanged evidence and guest resource limits.
