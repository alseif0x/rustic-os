<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native client and helper authority

This bounded #13 implementation connects the [authority decision](architecture/ADR-0002-authority-and-delegation.md) to a real client C and helper H running in separate RusticOS processes. The owner provisions both explicitly. C can read and replace selected A; H can only read A. Both are denied B. The shell retains its own file and control channels. No model, network, MCP or visual automation participates.

## Session contract

Every file grant binds an authenticated PID, server endpoint, object scope, rights, fresh generation and optional absolute expiry. Its service-owned root identifies the revocation session. A standalone grant starts a fresh root; the private administrative channel can derive one helper level from a live root. The file service checks ancestry, rights and deadline attenuation and inherits the recovery subject from the root. A packet cannot choose that subject or its revocation root. The subject is the whole namespace of retained operations and receipts: every receipt, `INSPECT` lookup by retry tuple or identifier, and cancellation is answered only within the caller's subject, and an equal retry key in another subject is a different operation. The shell's owner client and the supervisor share subject `1`; a client that retains its own intents is given its own subject instead. The `TASKS_OWNER` role uses the identifier of the journal object it was granted, which is stable across relaunches on that journal and, because the supervisor refuses the role unless that object is above `1` and the service refuses a directory, can never be the owner subject. A helper cannot create another helper, enlarge its scope or extend its lifetime. Both actors occupy the existing two utility slots; there is no new process/channel quota.

A grant may additionally carry one second object scope. The private administrative channel adds it in a separate step (`GRANT_SECOND_SCOPE`, words `[38, slot, generation, object, 0, 0, 0, 0]`) that names the grant just issued by slot and generation; it never changes the peer, rights, subject, deadline or revocation root, and it is refused on a slot that is not its own root, on a grant that already has one, and on a second attempt. The added object must be one live file disjoint from the primary scope: a directory is refused even when the two subtrees are disjoint, it may not be the primary scope, contain it, or be contained by it, and a grant whose primary scope is the whole volume may not carry one. Both scopes are then checked independently with the same rights, so neither can extend the other, derivation always drops the second scope, and expiry, revocation, root death and slot reuse fence both at once. Only the supervisor's `TASKS_OWNER` role (a native tasks client over its task document and its own journal record) requests this; every other role still passes its second object identifier without receiving any authority over it. Portable file-service tests cover the added reachability, the rejected installations, the refused directory and the shared fencing. Guest execution of the role is exercised by `python3 tools/tasks_owner_test.py`, on its own disposable two-boot volume, and by the `tasks.owner` phase of `python3 tools/terminal_test.py`; the evidence is `artifacts/tasks-owner-test/tasks-owner.json` and the `tasks.owner` object of `artifacts/terminal-test/terminal.json`. Both run the child under its two scopes through committed and proven edits, a revoked grant that refuses the apply and the record query alike, process death with a retained intent, which a successor on the same journal observes, queries without resubmitting and finally discards with an explicit forget, and the launch refusals: a journal that is the target document, a journal that does not exist, and a directory, which passes the supervisor's own check and is then refused by the service as a second scope, so the already installed root is withdrawn and the job ends with a service error, no child and an immediately reusable slot. The shell also refuses its own record object `/config/tasks-intent` as a journal, because one client owns one record: two clients sharing it would each treat the other's unresolved intent as their own.

Because the addition is a second exchange, the root exists for a moment while the child is still dormant. A supervisor launch therefore ends in one of two ways only: the child is started with that root, or the root is withdrawn. If the second exchange is refused, answers something other than the generation it was given, cannot be decoded, or the launch job reaches its deadline while it is outstanding, the supervisor sends the ordinary slot revocation (`REVOKE`, words `[33, slot, 0, 0, 0, 0, 0, 0]`) and only then reports the failure; the same withdrawal covers a derived helper whose parent session died between the reply and activation, and an activation that could not start the child. An expired job is given one bounded extension for that single exchange and still reports a timeout. A refused or unanswered withdrawal leaves the supervisor degraded, because a root bound to a dead process would otherwise remain installed until the slot is reused. An accepted installation whose reply is otherwise unusable is withdrawn as well, since the withdrawal addresses the slot and needs no generation. The ordered words and every transition are covered by host tests of the supervisor's grant sequence; the guest scenarios drive the successful order and one withdrawal, the directory second scope refused by the service.

Revoking either member fences the whole session. The supervisor first disables fresh helper issuance locally, then requests a single file-service transition which clears member rights and staging. The request returns without waiting for that transition. Poll `revocation PID` to distinguish requested, unconfirmed and fenced access, independently of unknown/settled/recovery-required effects. A late authenticated acknowledgment can settle the original request. A missing reply leaves issuance disabled and never means successful takeover. See [stopped-service control](TAKEOVER.md) for the retained status and recovery contract.

The file service processes one request per client per pass, with a separate admin channel and bounded pending replies. It checks expiry at each request's actual admission time. Revocation runs between serialized requests: previously admitted I/O may complete before the acknowledgment. An uncertain volume produces `effects=recovery-required`, even though access is fenced. Reconcile retained [receipts](FILE-RECOVERY.md) after remount rather than assuming a failed or revoked call had no effect. Completed edits are never undone by revocation.

Queued requests are rechecked after the fence. The service discards its undelivered member replies; responses already copied into an IPC queue may still describe operations admitted before revocation. Receiving an older response creates no new authority. A fresh generation rejects old request contexts. The implementation does not retract data already delivered to a process.

Replacing or detaching a root fences its helper. The supervisor also observes root death through its separate control endpoint, so moving the file endpoint to H cannot hide C's exit. The kernel move invalidates C's old handle token and preserves the channel; the file service still authenticates the original C PID. H's use of the moved token is denied. Successful file delegation requires a separately provisioned, checked helper grant.

Service restart ends all actors and grants, remounts storage and issues fresh owner bindings. No utility process, lease or helper is restored automatically. The existing persistent `helpers=explicit` policy remains mandatory; malformed policy disables new utility grants while manual repair remains available.

## Storage-sourced launch topology

In `mode=terminal-v7` the owner can start the one child the supervisor staged from V7 storage ([storage-sourced images](NATIVE-RUNTIME.md#starting-the-staged-child-control-only)). What that child receives is supervisor policy, not a kernel rule: the kernel only enforces that the supervisor starts a dormant child it owns, and the manifest's feature bits were admission, never a grant. The supervisor issues exactly one topology, control-only: a single private channel between the supervisor and the child, carrying the role message and the child's report. It issues no file endpoint, file-service peer, grant generation, block grant or console, so the child has no file scope, rights or recovery subject, and `permissions` shows zero scope, rights and expiry.

The policy (`rustic_supervisor::storage_launch`, host-tested) accepts only the manifest identity `rustic.utility` and only the roles that need none of the withheld authority (FINISH, FAULT, SPIN), and requires the manifest to request `ipc`, which the issued channel implies. Other identities, including the `file-server` image the same request can stage, are refused before any role is considered. The identity is the name the manifest declares, bound to the ELF bytes by SHA-256; it is not an authenticated publisher. The started child stays in the single storage slot rather than a utility slot, and the owner stops it with the same `kill`/`reap` path. A refused start leaves the child dormant without an endpoint.

## V7 tracked-write authority

In `mode=terminal-v7` the file service accepts exactly three grant profiles: read-only (rights `1`, subject 0), tracked write (rights `7` = read, write and inspect, with a nonzero subject) and admission (rights `15` = tracked write plus cancel, with a nonzero subject; see [V7 admission authority](#v7-admission-authority)). It refuses any other rights value, a write or admission grant without a subject and a read-only grant with one, all as `Invalid`. The subject is the retry scope the service persists in every record the grant creates, so exact retries and conflicts are decided per subject, workspace, epoch and key. It is service policy, not a kernel identity.

The supervisor's policy is fixed. Its own owner binding stays read-only with subject 0, and it uses that binding to read the pinned pair it stages. The shell receives rights `15` (the admission profile; rights `7` before the V7 admission increment) and subject 2, scoped to the workspaces root. Subject 2 is deliberately distinct from subject 1, which `rustic-volume seed7` uses for the host provisioner's records, so the shell can neither replay nor inspect them. Lookups by operation ID or retry key answer only within the grant's subject, and a record neither of whose identities (workspace, object) lies inside the grant scope is `OutcomeUnknown`, the same answer as a missing one. Receipt parts after offset 0 are served only to the slot that completed or looked up the operation. Revoking, detaching, expiring or replacing a slot aborts its open stage without I/O before any later request is admitted, so a revoked client cannot commit a staged write. A write that already committed is never undone. No utility, staged child or helper receives V7 write authority.

Revoking the shell's binding is owner policy, not a shell right. Only the owner, through the shell's private supervisor channel, can start the `REVOKE_SHELL_V7` job (`[38, 0, 0, 0, 0, 0, 0, 0]`), and the supervisor accepts it only in the V7 profile, with a running and healthy file service and no other owner job. The job names no slot, subject or rights: the supervisor sends the administrative `REVOKE` for the shell's slot 0, and only after the service confirms it does it connect a new channel and grant it the same fixed shell policy (`rustic_supervisor::shell_binding`, host-tested) under a fresh context. The service closes the old endpoint before replying, so the old binding cannot reach the new grant, and the transfer it held has no stage left to commit. The new binding is reported with the restart binding words, which the shell adopts only when they come from a newer job than its current binding. If the revocation is refused, the old binding stays; if anything fails after it, both ends of the unreported channel are closed, the supervisor is marked degraded and the shell has no file binding until `restart files` issues a fresh incarnation and binding. `python3 tools/v7_write_test.py` exercises the job in the guest during a 512 KiB transfer; see [owner revocation during a transfer](FILES-V7-WRITES.md#owner-revocation-during-a-transfer). During an admission publication the service applies the revocation at once but acknowledges it only after the publication settles; `python3 tools/v7_authority_test.py` exercises that in the guest.

Retention maintenance is owner policy too, never a client right and never automatic. Only the owner, through the shell's private supervisor channel, can start the `MAINTAIN_V7` job (`[39, 0, 0, 0, 0, 0, 0, 0]`); the supervisor accepts it only in the V7 profile, with a running and healthy file service and no other owner job, and it names no slot, subject, epoch or record. The supervisor sends the administrative `MAINTAIN_RETENTION` request (`44`) on its private bootstrap channel to the file service; the V7 service handles that opcode only there, so no file grant, including the shell's tracked-write grant, can reach it, and a client packet carrying it changes nothing (host-tested). Starting the job is the owner's declaration that clients have resolved the current-epoch outcomes they need, because storage cannot tell whether a completed reply was observed. The service still refuses with `Busy`, changing nothing, while any transfer, stage or unresolved admission is open, so maintenance never discards a retry in flight or an outcome that is not yet terminal. A completed maintenance drops every terminal record and expires the old epoch for every subject, including the host provisioner's subject 1 seed records: old-epoch retries and lookups answer `ExpiredEpoch`. `python3 tools/v7_retention_test.py` exercises the job in the guest; see [owner retention maintenance](FILES-V7-WRITES.md#owner-retention-maintenance).

## V7 admission authority

The V7 admission increment widens the shell's V7 authority by exactly one right: `CANCEL`, so the shell can durably cancel its own subject's admissions within its workspaces scope. It gains no new subject, scope, service slot or owner request. The supervisor now grants the shell rights `15` (`rustic_supervisor::shell_binding::RIGHTS`, host-tested) instead of `7`, and the file-service ready report advertises admissions (word 5 bit 1) before the supervisor accepts it. The same fixed policy is reissued after `REVOKE_SHELL_V7` and `restart files`.

The service policy mirrors the v5 direct admission path ([V7 staged admissions](FILES-V7-ADMISSIONS.md)):

- Every admission request needs a nonzero subject. Staging and ACCEPT need write and inspect. GET, RETRY and OBSERVE need inspect. EXECUTE needs inspect, and for an admitted record also write access to the file within the grant scope, checked when the request is served (a removed target is `Denied`; CANCEL still resolves the admission). CANCEL needs `CANCEL` and inspect, because its reply discloses the admission's status.
- Admissions exist only within the grant's subject, and a record outside the grant scope is answered as missing. Replies disclose only the minimal admission status; the completion receipt needs a separate inspect-authorized lookup.
- The executor is the admitting subject, acting under whatever authority it holds when it asks. An admission is not authority: it never executes by itself, on retry, lookup or remount. A version change since admission refuses execution (`Version`) and the record stays admitted until the client cancels it; the service records no cause it did not observe.
- The tracked-write profile (`7`) remains accepted: it can admit and execute but its CANCEL is `Denied`. Read-only grants and utilities, helpers and staged children get no V7 admission authority.
- Owner control continues while an admission publication is in flight, mirroring v5 ([owner control during a publication](FILES-V7-ADMISSIONS.md#owner-control-during-a-publication)). The owner may revoke or detach clients between polls; grants, maintenance and other owner requests are `Busy` until settlement. A caller that loses its authority (revocation, detach or expiry) before the header is submitted gets no durable effect: its execution is recorded cancelled with cause `AuthorityLost` by the service's own housekeeping publication, which needs no client authority, and its acceptance leaves no admission. After the header the effect stands; a new admission is then retired with `AuthorityLost`, and a settled execution or cancellation is reported `Uncertain`. The revocation is acknowledged only after settlement, so `REVOKE_SHELL_V7` re-grants the shell only once the outcome is durable. Retention maintenance stays `Busy` while any admission is unresolved.

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

The [explicit admission API](FILE-ADMISSION-API.md) adds the independent CANCEL right (bit 8). It permits scoped terminal prevention/status acknowledgement, without implying READ, WRITE or INSPECT. The private owner shell receives all four rights; existing utility/helper provisioning retains its narrower grants.
