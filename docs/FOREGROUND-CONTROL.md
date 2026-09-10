<!-- SPDX-License-Identifier: Apache-2.0 -->

# Interruptible owner control and admitted I/O recovery

This #13 increment extends [stopped-service takeover](TAKEOVER.md) to foreground waits and writes already submitted to the real reference device. The shell remains usable after interrupting a file wait. Supervisor provisioning, policy loading and restart advance through bounded polling steps while independent owner requests continue.

## Foreground behavior

Ctrl-C cancels an input line at the prompt. During a file RPC it stops waiting, returning `Interrupted` for an interrupted non-durable request or conservatively `Uncertain` for a potentially durable mutation. Earlier steps in a command may already have changed the disk. Neither result promises cancellation or rollback. An abandoned admitted reply poisons that file binding; obtain a fresh binding with `restart files`, then reconcile file contents and retained [receipts](FILE-RECOVERY.md) before retrying.

The shell owns a separate 1024-byte queue for input received while waiting. Normal typeahead is replayed at the next prompt. Ctrl-C discards the buffered partial input; subsequent bytes remain available. Overflow interrupts the wait and discards input through the next newline, so a truncated suffix cannot execute as a command. This is not a background shell or a general async executor.

The SDK `Progress` trait lets the caller choose its waiting policy without importing a console into transport code. Existing clients use `Blocking`; the shell uses its console-owned implementation. `files::Client::submit`/`poll` retain one original packet and validate the authenticated RPC peer, correlation, opcode and context. A rejected `Busy` admission may be attempted again; an admitted request is never silently resubmitted. Full logical service-v1 operation IDs, cancellation and event streams remain separate work.

## Supervisor jobs

`restart files async` returns a job ID immediately. `job-status [ID]` queries progress or its retained result; omission selects the latest job. Plain `restart files`, grant provisioning, receipt rotation and diagnostic arming retain convenient foreground waiting. Ctrl-C during a job wait prints its pending ID and returns control; the job continues. Query the same job to collect its outcome.

One management job may run at a time, with two completed results retained in memory. IDs increase within the supervisor lifetime; unknown or evicted IDs are denied. Restart can supersede an earlier job. Result status 6 means superseded, and does not erase a mutation already submitted by that job. No old utility session is restored automatically.

| Restart phase | Work |
| --- | --- |
| 1 | Accepted |
| 2 | Stop the old service and retain its PID while kernel I/O drains |
| 3 | Reap completed retirement and provision a new service incarnation |
| 4 | Await its authenticated ready message |
| 5 | Obtain the owner grant |
| 6 | Load or initialize the owner policy |
| 7 | Obtain a fresh shell grant |

The owner can query processes, memory, revocations, job status and diagnostics throughout these phases. A newly provisioned helper stays dormant until the file service acknowledges its grant; activation rechecks its parent session. Pending helpers participate in root revocation accounting. Only a successfully collected restart result newer than the shell's adopted job can replace its binding. Querying an older retained result cannot restore an old token.

Jobs have a 1000-PIT-tick deadline (nominally ten seconds on R0). Timeout or failure reports an error, releases partial provisioning and leaves recovery explicit. A failed mount retains its service PID when necessary for a later drain/retry. The shell starts before the initial mount: Ctrl-C can leave its initial job wait and reach owner control even if the disk or policy fails. Failed startup never reformats an existing image automatically.

Private owner protocol additions are `JOB_STATUS=19`, `HOLD_IO=20` and `IO_STATUS=21`. Pending replies contain `[5, id, kind, phase, service_pid, pending_io, 0, 0]`; completed queries contain `[0, id, kind, result_status, value, token, generation, 0]`. Kinds use the original operation opcode. These experimental binaries are built together; this is not a stable cross-version service-v1 wire contract.

## Real-submission diagnostic

`hold-io SKIP TICKS` arms one completion-observation hold for the current file-service PID. It skips 0–16 of that process's write/flush submissions, then withholds kernel observation of the selected request for 1–500 PIT ticks. Reads and other owners do not consume the skip. `io-status` reports the arm, actual request ID and deadline. Re-arming an active hold is rejected.

The driver really submits and notifies QEMU; only subsequent completion observation is delayed. QEMU may finish the write or flush while RusticOS still reports `pending_io=1`. This deliberately exercises the uncertainty window, not a physically frozen controller or power failure. Kernel-owned copied sector data and DMA remain alive until completion/reset. A retiring service cannot be replaced before that outstanding request drains. The ordinary driver timeout/reset path remains unchanged after the observation hold expires.

This diagnostic is compiled with the existing `sdk-test` catalog configuration and is available only through trusted supervisor control. Scoped utilities are tested to receive denial. No new authority comes from a normal block or file grant. It is a reference acceptance mechanism, not a general device policy API.

## Native evidence and limits

`terminal-test` interrupts an indefinitely stalled read, checks typeahead and overflow, rejects stale restart-result adoption, and restarts while C's actual first data write is pending. It verifies conservative C/H revocation, owner progress during drain, unchanged A/B after remount, fresh C writing A, and complete frame/process/channel/I/O reclamation. `terminal.json.management` records the measured interruption time and assertions. The two-host-second interruption check is a regression tripwire, not a latency guarantee.

`recovery-test` adds two cases, each followed by a separate VM boot:

| Held actual submission | Observed after interrupted reply, explicit restart and reboot |
| --- | --- |
| First data write (`SKIP=0`) | Original content/version and `OutcomeUnknown` for the new receipt |
| Final publication flush (`SKIP=16`) | New content/version and its committed receipt |

Both cases require pending I/O, independent owner progress, C/H fencing with recovery-required effects, denial of old actors and an unchanged second file. An independent Python decoder checks on-disk checksums, versions, contents and receipts. Combined with the existing lost-reply, five QEMU EIO-cut and upgrade groups, recovery acceptance covers nine groups across 18 actual VM boots. The user terminal image is not used by acceptance.

Pure tests cover bounded job identity/history, input interruption/overflow and hold selection/deadlines. Module responsibilities separate transport, shell input/control, supervisor job lifecycle/provisioning/policy, kernel observation and host fault acceptance. No new unsafe block, dependency or resource quota is introduced. Implementer review and automated checks are not an independent audit.

Remaining work includes full service-v1 operation/reference/regrant integration, the other authority routes in #13, broader device/cache failures and repeated topology/latency measurements in #20. These results establish a bounded native owner-control and disk-recovery path, not general cancellation or production filesystem guarantees.
