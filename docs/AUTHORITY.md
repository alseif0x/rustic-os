<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native client and helper authority

This bounded #13 implementation connects the [authority decision](architecture/ADR-0002-authority-and-delegation.md) to a real client C and helper H running in separate RusticOS processes. The owner provisions both explicitly. C can read and replace selected A; H can only read A. Both are denied B. The shell retains its own file and control channels. No model, network, MCP or visual automation participates.

## Session contract

Every file grant binds an authenticated PID, server endpoint, object scope, rights, fresh generation and optional absolute expiry. Its service-owned root identifies the revocation session. A standalone grant starts a fresh root; the private administrative channel can derive one helper level from a live root. The file service checks ancestry, rights and deadline attenuation and inherits the recovery subject from the root. A packet cannot choose that subject or its revocation root. A helper cannot create another helper, enlarge its scope or extend its lifetime. Both actors occupy the existing two utility slots; there is no new process/channel quota.

Revoking either member fences the whole session. The supervisor first disables fresh helper issuance locally, then requests a single file-service transition which clears member rights and staging. The request returns without waiting for that transition. Poll `revocation PID` to distinguish requested, unconfirmed and fenced access, independently of unknown/settled/recovery-required effects. A late authenticated acknowledgment can settle the original request. A missing reply leaves issuance disabled and never means successful takeover. See [stopped-service control](TAKEOVER.md) for the retained status and recovery contract.

The file service processes one request per client per pass, with a separate admin channel and bounded pending replies. It checks expiry at each request's actual admission time. Revocation runs between serialized requests: previously admitted I/O may complete before the acknowledgment. An uncertain volume produces `effects=recovery-required`, even though access is fenced. Reconcile retained [receipts](FILE-RECOVERY.md) after remount rather than assuming a failed or revoked call had no effect. Completed edits are never undone by revocation.

Queued requests are rechecked after the fence. The service discards its undelivered member replies; responses already copied into an IPC queue may still describe operations admitted before revocation. Receiving an older response creates no new authority. A fresh generation rejects old request contexts. The implementation does not retract data already delivered to a process.

Replacing or detaching a root fences its helper. The supervisor also observes root death through its separate control endpoint, so moving the file endpoint to H cannot hide C's exit. The kernel move invalidates C's old handle token and preserves the channel; the file service still authenticates the original C PID. H's use of the moved token is denied. Successful file delegation requires a separately provisioned, checked helper grant.

Service restart ends all actors and grants, remounts storage and issues fresh owner bindings. No utility process, lease or helper is restored automatically. The existing persistent `helpers=explicit` policy remains mandatory; malformed policy disables new utility grants while manual repair remains available.

## Try the deterministic mission

Use the actual PIDs printed by `session` and `helper`; 4 and 5 below are examples. These actors are diagnostic native clients from the fixed catalog, not an application loader or a general agent framework. Their prepared replacement is the fixed text `session client edit`.

```text
write a before
write b untouched
session a b
helper 4 a b
act 4 read
actor-status 4
act 5 read
actor-status 5
act 5 stage
actor-status 5
act 4 stage
actor-status 4
act 4 commit
actor-status 4
cat a
act 4 read
actor-status 4
act 4 stage
actor-status 4
write a human-edit
act 4 commit
actor-status 4
revoke 4
revocation 4
act 5 read
actor-status 5
cat a
kill 4
reap 4
kill 5
reap 5
```

`act` returns pending after command admission. Repeat `actor-status PID` until complete before the next action; repeat `revocation PID` until fenced when testing revocation. Completion reports file status separately from owner-command delivery: 0 success, 13 version conflict, 17 denied, 18 revoked, 19 expired. `read` records the observed version, byte count and denial of B; `stage` uses that version, and `commit` submits the prepared ordinary replacement. These fixture writes are untracked: use `replace` and `receipt` for recoverable operations. Read/write rights and receipt inspection remain distinct.

`session A B TICKS` supplies an optional lifetime (100 PIT ticks/second); H inherits the same absolute deadline. `act PID flood` fills that actor's file queues for 20 ticks without reading replies. Its result includes sent and blocked counts. `act PID drain` consumes fixture replies for 100 ticks and reports how many were revoked. Drain before another normal file RPC. `move-check C H` moves C's file endpoint to H and tries it there; `act C stale` checks rejection of C's old handle. This deliberately consumes C's ordinary access path until the actors are stopped and freshly provisioned.

## Native evidence and limits

The existing `terminal-test` now executes the C/H mission in addition to its manual-shell cases. The first integrated run completed 240 commands across the first boot and verified persistence with a second VM. The command count may vary with process-exit polling. It checks:

- C reads/writes A; H reads A but cannot stage a write; both are denied B and privileged process/move control.
- Rejected out-of-scope helper provisioning reclaims the dormant process and channels; H cannot request descendants.
- An owner edit between stage and commit causes a version conflict and survives.
- Both file request/reply paths saturate; the owner still writes and revokes both members. Queued requests receive Revoked, staged data is discarded, and fresh calls remain rejected.
- A moved handle cannot lend C's grant to H; the old token is invalid; explicit revocation and root death after movement both fence H.
- Deadlines are inherited; expired grants fail; service restart rejects old actors and requires fresh issuance.
- File bytes and checksums are independently inspected on the host. Frame/process/channel/I/O counts return to the same in-VM baseline.

The five existing QEMU EIO cuts in `recovery-test` additionally keep a C/H session present during a failing durable write. The owner receives `access=fenced ... effects=recovery-required`, H loses access, manual runtime inspection works, and remount plus receipt/content inspection resolves the actual effect. These are real guest errors, not host policy mocks.

`terminal.json` includes an `authority` object with observed control/write elapsed times, occupancy and independent disk hash. One local sample measured 0.173 seconds for the owner write under saturated actor queues and 0.006 seconds for revocation. These are observations from one run, not latency guarantees or #20's statistical baseline. The topology peaks at five processes/eight channels and uses unchanged 64 KiB guarded stacks and two staging buffers. Two independently granted writers can still exhaust both staging buffers; the owner control channel remains separate, but universal write availability is not promised.

Four portable policy tests cover subset/identity/lifetime restrictions, root fencing/staging and death/regrant/moved-peer checks and delayed old-root revocation after slot reuse; native tests establish actual guest execution separately. No new dependency or `unsafe` boundary is introduced. The kernel only exposes its existing move mechanism through authenticated supervisor control; it does not import file policy.

The [next native increment](TAKEOVER.md) adds asynchronous revocation/actor status, owner progress during a stopped file service, late acknowledgment and explicit restart recovery. Remaining #13 work includes converting the other bounded foreground waits, asynchronous admitted-device takeover, full service-v1 reference/regrant integration and wider authority routes. General delegation graphs, persistent session restoration, networking/credential authority and adapter conformance remain future work. Review is by the implementing agent with automated checks, without an independent audit.
