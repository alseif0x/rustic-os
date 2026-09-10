<!-- SPDX-License-Identifier: Apache-2.0 -->

# Owner control during a stopped file service

This #13 increment keeps revocation and deterministic actor control independent of a file-service reply. It extends the [native client/helper mission](AUTHORITY.md). A real ring-3 service can stop reading all its channels while the owner inspects processes, requests revocation, checks its status, kills/reaps utilities and starts non-file utilities. The prompt and `pwd` use the last validated directory path; printing a prompt requires no file request.

## Request, acknowledgment and effects

`revoke PID` disables local issuance for every live member of that root and schedules one admin request. It returns immediately with a retained status; it does not wait for the service to acknowledge. `revocation PID` queries the same record, including after that actor is reaped.

| Access state | Established fact |
| --- | --- |
| `requested` | Issuance is disabled locally; service fencing has been requested. Existing service authority is not yet confirmed revoked. |
| `unconfirmed` | No valid acknowledgment arrived within 200 PIT ticks, or the channel/acknowledgment failed. Effects remain unknown. This timeout is an observation, not cancellation. |
| `fenced` | The original service acknowledged the root fence, or the old service was killed and successfully reaped after outstanding kernel I/O settled. |

Effects are separate: `unknown`, `settled` or `recovery-required`. A valid service acknowledgment records its volume sequence, discarded staging count and uncertainty flag. Successful revocation never undoes an earlier write. Forced service retirement preserves `discarded_staging=unknown effects=recovery-required`; remount alone cannot invent the original operation's result. Reconcile retained [receipts and file contents](FILE-RECOVERY.md).

The supervisor retains at most two records, matching its two utility slots. Completed records may be replaced by later sessions; this is bounded live recovery state, not a durable audit log. Each record binds the original service PID and incarnation-local root. The RPC adds an authenticated peer and unique correlation; a late acknowledgment completes only its original request. An invalid acknowledgment cannot become successful takeover. Root revocation is idempotent even after slots are detached or reused; a delayed old-root request cannot revoke a new root in the same slot.

While any revocation is pending or the admin binding is unavailable, fresh file grants and receipt rotation return Busy. `permissions` shows the locally requested rights; zero rights there is not evidence that the service has fenced access. `services` reports `control-pending`. Existing owner operations that require no file access remain available. Root death schedules the same fence, including when an actor command is pending; reaping preserves the root long enough to record it.

## Asynchronous actor commands

`act PID ACTION` and `move-check C H` return `actor state=pending` once their command is admitted to the control queue. Poll `actor-status PID` until `complete` before issuing another action to that actor. File status is meaningful only at completion: 0 success, 13 version conflict, 17 denied, 18 revoked, 19 expired. A pending actor cannot accept a second command. After 200 ticks, status becomes `unconfirmed`; the supervisor still accepts a valid late reply to that original command. These are fixed catalog diagnostics, not general asynchronous service-v1 operations.

The private owner protocol adds ACT_STATUS (16), REVOCATION (17) and STALL_FILES (18). REVOKE (7) returns eight words: status, access phase, member count, discarded staging count (`u64::MAX` for unknown), effect state, sequence, root and service PID. Access phases are 1 requested, 2 unconfirmed, 3 fenced; effects are 0 unknown, 1 settled, 2 recovery-required. ACT/ACT_STATUS return status, actor phase, then five report words (file status/value/other/control-denied/version), with the final word reserved zero. Actor phases are 0 idle, 1 pending, 2 complete, 3 unconfirmed. These experimental clients and services are built together; the changed replies are not backward compatible with earlier binaries and do not claim the separate logical service-v1 contract.

## Reproduce a stopped service

Use a disposable acceptance disk and the actual PIDs printed by `session` and `helper` (4/5 here are examples). Poll actor status to completion after each initial action.

```text
write a before
write b untouched
session a b
helper 4 a b
act 4 read
actor-status 4
act 4 stage
actor-status 4
stall files 600
act 4 commit
revoke 4
revocation 5
services
mem
pwd
echo owner-control
actor-status 4
revocation 4
```

`stall files TICKS` is an explicit owner-only fault diagnostic, reached through the authenticated supervisor/admin channels. It acknowledges arming, then the real service spins without reading requests. Timer preemption continues. The range is 0–1000 ticks (100 ticks/second); zero stalls indefinitely until service restart or VM exit. It is not exposed to utility grants. Once a finite stall ends, the queued root fence is processed before the queued actor commit: staging is discarded, the late fence acknowledgment settles the record, and that commit reports Revoked. Without a fence, a finite diagnostic leaves issuance disabled until explicit recovery.

For an indefinite stall, `restart files` kills old utilities and the file service, checks pending kernel I/O and requires successful reap before remount/fresh owner bindings. If device work is still pending, the [restart job](FOREGROUND-CONTROL.md) retains the old service PID and reports its drain phase while owner control continues. Old actors and contexts never restart automatically. The old retained record remains conservative about effects; newly issued sessions get a fresh service binding.

## Validation and remaining work

Run the established checks and native scenarios:

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode terminal-test --timeout 60
python3 tools/boot.py run --mode recovery-test --timeout 60
```

The terminal driver uses a six-second stall to prove requested/unconfirmed states, independent owner progress, denial of new helper issuance, a late acknowledgment and rejection of the queued commit. It then stalls indefinitely, kills/reaps the client while its command is pending, observes automatic session revocation, exercises utility-slot reuse, restarts files, rejects the old actor and verifies fresh access. Independent disk inspection checks A and untouched B; process/frame/channel/I/O counters return to baseline. A second VM verifies persistence. The five existing native EIO cuts also use asynchronous revocation and still require fenced access with recovery-required effects.

`terminal.json.takeover` retains the stall duration, state deadline, owner command count, maximum observed command time, disk hash and reclamation flags. Each measured independent control command must finish within two host seconds, a regression tripwire against the previous ten-second nested waits. This is not a product latency SLO or #20's repeated statistical baseline. Polling and host scheduling vary the total command count.

Pure tests separately cover RPC correlation/admission/poisoning, no false completion after a missing/invalid acknowledgment, late reply identity, monotonic retirement, and old-root revocation after slot reuse. SDK transport, supervisor facts/coordination, file enforcement, shell presentation and host fault acceptance have separate modules. The kernel and its quotas are unchanged; no new unsafe boundary or external dependency is introduced.

**Extended boundary:** the stopped-service cases above stop before device admission. [Foreground control and admitted I/O recovery](FOREGROUND-CONTROL.md) now covers interruptible file waits, incremental provisioning/policy/mount jobs, and real write/flush submissions across restart and reboot. Full service-v1 operation/reference/regrant semantics and repeated #20 topology measurements remain open. Review is by the implementing agent and automated checks, without an independent audit.
