<!-- SPDX-License-Identifier: Apache-2.0 -->

# Owner control across durable admission

The service library now drives durable admission, file execution and terminal cancellation through the same pollable storage owner. This connects the [format-4 storage facts](FILE-ADMISSION.md) to current service authority for #12/#13. The typed controller runs in a native ring-3 acceptance application with real block I/O. The production file-server IPC still serves `completed_operations`; accepted/result framing and a delegated `operations.cancel` endpoint remain future work.

## Storage mechanism

`Volume::prepare_admission` and `prepare_cancellation` return a `Publication<AdmissionStatus>` without issuing I/O. The existing synchronous `admit_replace` and `cancel_admission` functions drive those same guards to completion. Metadata-only publication uses the existing encoder and 14 commands, omitting the two data writes and data flush used by a 17-command file replacement. It neither writes a speculative file extent nor changes the format, capacity or migration policy.

Preparing a guard exposes no acknowledged ID or terminal state. `result()` remains empty until the header and final flush succeed. Before header submission, cancellation drains an outstanding scratch command and abandons the transition. Abandoning an admission means it was never accepted; abandoning a cancellation leaves the operation `Admitted`. Once the header may have been submitted, settlement is mandatory. Error, pending abandonment, forgotten guards and adapter unwind fence the live volume. Remount selects storage facts, never permissions or execution.

## Service policy

`Caller` contains a transport-bound slot, authenticated peer and current context. A model or client payload must never supply that binding. The service derives the durable subject from the live grant. `Clients` owns grants, roots and staging separately from the exclusively borrowed volume. The bounded owner callback can revoke, detach and expire clients while a disk command remains pending; issuing a grant or changing its subject, scope or rights requires the whole server.

| Typed service entry | Required current authority | Behavior |
| --- | --- | --- |
| `admit_with` for a new retry | Inspection plus write access within the actual workspace | Persist complete arguments and return `Admitted`; leave file bytes/version unchanged |
| `admit_with` for a retained retry | Inspection of its stored subject and scope | Verify identical arguments and return historical state with no I/O; never execute or cancel it |
| `admission_status` | Inspection of the stored subject and scope | Read status only; hidden and missing identities have the same result |
| `execute_admission_with` for admitted work | Fresh inspection and write access | Recheck the current workspace/version and explicitly execute the retained payload |
| `execute_admission_with` for a terminal record | Inspection | Return immutable historical status without I/O |

Every controlled storage poll rechecks the live caller, context and expiry, including the final result boundary. Revoking a root reaches its helper; an unrelated root cannot stop the operation. Rights and scope cannot be enlarged through the callback. A restarted service has no grants until its owner issues them again. An inspection-only grant can recover identity but cannot execute admitted work.

## Revocation and terminal cleanup

| Revocation boundary | Required settlement | Observable result |
| --- | --- | --- |
| Before admission header submission | Drain the submitted scratch command; abandon admission | Denial, no admitted record or file effect |
| Admission header may be submitted | Settle admission, then persist terminal cancellation | Denial only after terminal cleanup; file unchanged |
| Before file header submission | Drain the submitted command, then persist terminal cancellation | Denial with retained `Cancelled`; old file/version remains |
| File header may be submitted | Settle the immutable file/version/receipt transition | `Uncertain` under revoked authority; fresh authorized lookup finds `Committed` |
| Any required settlement fails | Fence the live volume | `Uncertain`; explicit recovery must determine the selected state |

Terminal cleanup is a private service policy after owner revocation, expiry or detach has prevented a newly admitted operation or its explicit execution. It continues polling owner control even if the original client disappears, but must finish the cancellation write. It does not require preserving that client's now-revoked grant and cannot publish a file effect. A historical retry cannot invoke this cleanup path. A crash before cancellation becomes durable may leave `Admitted`; no automatic restart follows.

This cleanup is not delegated cancel authority. A future public cancellation method still needs a separate right, target validation and accepted/result IPC contract. Inspection or file-write rights must not silently become that permission. No new right bit, opcode, SDK method, catalog operation or public asynchronous profile is introduced here.

## Validation

Run the configured commands from Ubuntu with the pinned toolchain:

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode block-user --timeout 45
python3 tools/boot.py test --timeout 45
```

Host service tests revoke at all 14 pending admission positions and all 17 pending file-publication positions, require repeated control while completion is withheld, and inject errors at each of the 14 terminal cleanup commands. They verify current grants, restart, inspection-only rejection, hidden subjects/scopes, foreign peers, root/helper revocation, expiry, detach and read-only historical retries. Filesystem tests additionally abandon both metadata guards at every pending command and challenge drop, forget and adapter unwind. Existing torn-write and crash-cut tests now exercise the same metadata publication implementation used by polling. Host disk models do not establish physical power-loss behavior.

The existing two-VM `block-user` acceptance still uses four sequential applications and the same stack, event, process and device quotas. Its terminal volume now runs the service controller: revoke after admission-header submission, persist cancellation, then revoke another operation after file-header submission and settle its committed effect. The native adapter submits actual copied block requests and deliberately withholds completion observation across control callbacks. The pending volume admits work, then demonstrates inspection-only retry and rejected execution with zero writes. The second VM remounts both volumes with fresh grants; the independent host oracle requires the same files, versions, records and complete volume hashes. Evidence requires `service_control=1` and `fresh_authority=1` in addition to the original storage and reclamation checks.

These new native cases invoke the service policy library directly; their owner callback is a deterministic fixture, not new administrator IPC. Existing `recovery-test` separately retains its four real private-owner-IPC races and completed-profile regression. Connecting the new controller to accepted/result IPC, public cancellation, response-loss recovery and selected native EIO scenarios is still required before advertising an asynchronous service profile.

Development evidence includes a second-VM ring-3 stack fault at `0x7ffef1e8` in the pending-admission fixture. Separating its mount/setup frame from replay verification removed the cumulative buffer overlap. The corrected two-VM run passed in 39.780 and 8.026 seconds with the existing 64 KiB stack and 4,096-event limit. The first failed replay is retained as a failure, not counted as restart acceptance. Configured host validation passed 162 Rust tests, 146 runner tests and 34 contract tests, including formatting, Clippy and native builds.

## Implementation review

Filesystem admission construction, terminal cancellation, execution and migration have separate modules. The service separates caller authorization, the shared controlled-settlement loop and transition policy. Both crates forbid unsafe code. The acceptance application adds only a dependency on the existing service crate; external versions, notices and kernel dependencies are unchanged. No user disk is migrated automatically. Review is by the implementing agent and automated checks, not an independent audit.
