<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native tasks application

`tasks list PATH`, `tasks add PATH TITLE` and `tasks done PATH ID` use a separate Rust application inside RusticOS. The application owns document validation and edit planning. The supervisor grants it read access to the chosen file and relays complete, bounded results over private IPC. The owner client in the shell retains and applies an immutable plan through the existing file SDK; the child receives no write or console authority.

Try this in the native terminal:

```text
write todo "rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n"
tasks list todo
```

Expected result:

```text
7 [open] Review kernel
42 [done] Boot the OS
2 tasks
```

## Document contract

The ASCII document starts with `rustic-tasks-v1` followed by a newline. Each subsequent line contains a nonzero unsigned 32-bit task ID, a tab, `open` or `done`, a tab, and a title followed by a newline. IDs use decimal digits without leading zeros and must be unique within the document. The header alone represents an empty list.

The initial implementation accepts at most 16 tasks, titles of 1–24 printable ASCII bytes, and a file of at most 1,024 bytes. Unknown versions, duplicate IDs and malformed records fail validation; exceeding the task/title bound reports capacity exhaustion. The application validates the complete document before returning any rows. Listing does not modify the file or its version.

These bounds describe the first application slice, not target OS capacity. The application uses the existing file service and process/channel budgets. It is a separate ELF in the trusted embedded catalog; independently installing executables remains [#52](https://github.com/alseif0x/rustic-os/issues/52).

## Edit previews

The native application can calculate an edit without applying it:

```text
tasks preview add todo "Ship Rust"
tasks preview done todo 7
```

Both commands show the complete candidate list, the affected task ID, whether the bytes would change, and the source version used. The final line says `not applied`. Repeated previews read the current document afresh; they do not reserve a request identity or authorize a later commit.

Adding chooses one greater than the largest existing ID (1 for an empty document), preserves existing IDs/order and rejects ID exhaustion or capacity overflow. Completing a task changes only its state; a missing ID fails, and an already completed task produces an unchanged candidate. The app-owned pure planner retains immutable candidate bytes; the native path reads one version-pinned, hash-verified snapshot and returns validated candidate rows. The supervisor grants only READ for the selected file, and the shell owns syntax/presentation. Result cleanup and expiry are shared with listing.

## Applying edits and recovery

Enable writes once, explicitly upgrading persistent storage to at least format 3, then use ordinary task commands:

```text
tasks enable
tasks add todo "Ship Rust"
tasks done todo 7
tasks list todo
```

The owner-client mechanism is the shared `rustic-tasks-client` crate (`crates/tasks-client`): it owns the recovery record, the single submission and the outcome proof, while the shell keeps command syntax, help text and every terminal line; the record location is a constructor argument, either a path below the volume root or an object ID a launcher granted, so another semantic client can reuse the crate with its own record. The crate separates the two authorities it borrows: planning an edit needs the supervisor `Relay`, while retaining, submitting, proving, recovering and forgetting need the file `Authority` alone, so a client that may not address the supervisor can still apply a validated `Candidate` another process planned. Each mutation first collects the exact candidate bytes from the native planner and checks the original version again. The owner client reserves `/config/tasks-intent` for one immutable recovery record: volume lineage, selected workspace/resource, original version, retry epoch, command, affected ID and candidate bytes/hash. The record fits within the existing 1,024-byte file bound. Its atomic legacy replacement does not consume a tracked receipt. An exact version-pinned readback must succeed before any target submission.

The journal's committed metadata version becomes the initial retry key under the existing stable owner subject `1`. The file effect uses the existing completed-operation profile. A matching receipt must have a version greater than that journal version; an older diagnostic operation with the same numeric key cannot establish success. The client then independently reads the pinned committed version and compares the exact bytes before clearing the intent with a version-checked replacement. The idle journal remains as one empty file. Already-done commands make no writes and consume no receipt.

One unresolved intent blocks new task mutations. `tasks recover` queries the original retry tuple under the current owner binding and performs the same receipt/byte checks. It never resubmits or rebuilds the edit. After a disconnected file binding, run `restart files` first; that existing command revokes utility sessions. Missing or expired outcomes remain uncertain. If a human changed the target after the recorded commitment, the client reports the historical commitment but retains the intent because current bytes cannot be verified at that version.

Losing the journal commit response stops before target submission. After restart, the record may exist, but an absent outcome does not establish which instruction the earlier process reached. Recovery remains query-only. `tasks forget INTENT_KEY` deliberately discards that recovery record; it does not cancel or undo a submitted effect. The key must match the currently retained journal version. Corrupt records require explicit owner repair, not automatic interpretation.

The journal is owner-controlled application state, not a new kernel authority or tamper-proof ledger. Do not edit, delete or reuse it while an intent is unresolved. The identity guarantee follows surviving committed history of one volume lineage; cloning or rolling back an entire volume is outside this single-owner protocol. Other owner diagnostics share the two retained outcome slots and numeric key namespace. Collisions and expired epochs stay explicit; task commands neither rotate receipts nor silently expand capacity.

## Second native client (utility)

The utility application (`apps/utility`) is a second, separately implemented semantic tasks client. Under supervisor role `TASKS_OWNER` (15) it is launched with a two-scope grant — the target task document as `scope`, its own journal object as `other`, rights 7 under a recovery subject of its own — and it is the only role whose grant carries a second object scope. That subject is the journal object identifier itself, not the owner subject `1` the shell client uses, so the two clients never share a retry namespace; the supervisor refuses the role unless the journal object is above `1`, and the service refuses a second scope that is not a file. The two objects must be distinct: a journal aliasing the target would record recovery evidence inside the document that evidence protects. The child resolves no path, never creates its journal and passes `other` to `rustic-tasks-client` only as `Record::object(id)`. It holds no supervisor authority, so it cannot plan: planning stays in the read-only tasks application.

One ordinary shell command performs the whole hand-off:

```text
tasks-owner todo intent
tasks hand 3 add todo "Ship Rust"
tasks hand 3 done todo 7
```

`tasks hand PID add PATH TITLE` and `tasks hand PID done PATH ID` plan the edit in the same read-only native application `tasks preview` uses — through `rustic_tasks_client::plan`, the one planning path `tasks add`/`tasks done` also take, which retains nothing, submits nothing and reserves no request identity, so planning needs no record — and then relay the plan to the child at `PID`. The shell's own record is not read, written or created by the hand-off. On success the command prints one line, `handed pid=3 bytes=43 chunks=2 task=8`, and stops there: applying, recovering and forgetting are the child's, through `act PID tasks-apply|tasks-status|tasks-recover` and `tasks-owner-forget PID KEY`. If a step is refused, the shell prints `hand-off stopped pid=3 step=chunk error=100` (or `… step=chunk unconfirmed; query actor-status 3` when the step never completed) and reports the child's refusal in the shell's ordinary error vocabulary; whatever the child already collected stays with it until the next hand-off announces a fresh plan. A `PID` that is not a tasks-owner child is refused by the supervisor as an ordinary service error.

Because one client owns one record, `tasks-owner FILE JOURNAL [TICKS]` refuses a journal that is the target document and a journal that is the shell's own record object: two clients sharing one record would each treat the other's unresolved intent as their own. The check never creates the shell's record, so a shell that has not retained an intent yet has nothing to collide with.

The hand-off is owner-stepped. The owner plans an edit through the existing relay, then feeds the resulting candidate to the child one bounded message at a time: `TASKS_OWNER_EDIT` (the canonical `preview::Edit` words), `TASKS_OWNER_BEGIN` (candidate length plus the `preview::Summary` the plan was derived from) and one `TASKS_OWNER_CHUNK` per 32-byte chunk, packed by the product candidate codec. Chunk offsets are implicit in arrival order, and a begin always restarts collection. `ACT` verbs `TASKS_APPLY`, `TASKS_STATUS` and `TASKS_RECOVER`, plus `TASKS_OWNER_FORGET`, drive the rest. The supervisor refuses a step while the previous one is still pending, so the owner polls `ACT_STATUS` until the child's previous step is complete.

Applying uses file authority alone: the child retains the intent in its own journal object, submits once, proves the outcome and clears the record, under exactly the invariants the shell client follows. The candidate is released when an attempt concludes — a success, or one of the six canonical refusals that prove nothing was published — and kept otherwise, so an ambiguous outcome stays resolvable by `TASKS_RECOVER`. Each semantic client owns a distinct record and a distinct recovery subject: the shell keeps `/config/tasks-intent` under subject `1`, this client keeps the granted journal object under that object's identifier. So neither reads the other's retained intent, and their receipts stay separate too: the file service answers a retry tuple, an operation identifier and a cancellation only within the caller's subject, so equal journal versions in the two journals name two different operations instead of colliding or replaying each other's effect. A portable file-service test covers that separation; the relaunch identity follows from the journal object, and the guest scenario runs the same two edits through the shell client and through this one, on two different documents, and requires the second client's committed bytes to equal the shell's.

Only words 0..4 of a child reply are observable through `ACT_STATUS`, so every reply keeps its meaning inside them:

| Step | Reply words 0..4 |
| --- | --- |
| `TASKS_EDIT`, `TASKS_BEGIN`, `TASKS_CHUNK`, `TASKS_FORGET` | `[error, cursor, total, phase, 0]` |
| `TASKS_APPLY` | `[error, task_id, journal_key, applied, committed_version]` |
| `TASKS_STATUS` | `[error, phase, cursor, total, pending_journal_version]` |
| `TASKS_RECOVER` | `[error, recovered, journal_key, task_id, version]` |

`phase` is 0 idle, 1 an edit is stored, 2 collecting bytes, 3 a validated candidate is ready, 4 the last apply concluded and released its candidate. `applied` is 0 for a document that already showed the edit, 1 for a committed and verified effect, and 2 for a committed and verified effect whose intent could not be cleared; in that last case `error` still carries the cleanup failure, because the effect is durable while the record is not yet resolved. `recovered` is 1 only when a retained intent was matched to a committed effect.

Error `0` is success. A file refusal keeps the file ABI's own numbering (1–31), so a code means the same thing here as in every other reply the utility sends. A service or native-application refusal is `64 + code`. The remaining owner-client refusals are fixed: `100` document, `102` replacement not enabled, `103` journal, `104` an unresolved intent blocks the mutation. `105` is the client's own refusal: the step is not one the current phase accepts, and it changed nothing. The owner client also defines `101` capacity, but this client cannot report it: capacity exhaustion is raised by the planning relay alone (`crates/tasks-client/src/transport.rs`), and this client never plans. Retention exhaustion is a file-service refusal, so it arrives as the file ABI's own `Full` = `11`, after the intent was retained and therefore with the retained key in the reply.

The `journal_key` word of an apply reply names the retained record only when the intent was retained before the outcome was known: a refusal that happens earlier, such as a version pin that fails before retention or the `104` guard, reports `0` there, while a refusal after retention, including `Full`, reports the key that was cleared or that still stands. A `TASKS_RECOVER` that finds no record at all is not an error: it answers all zeros with `error 0` and `recovered 0`; only `tasks-owner-forget` on that same absent record answers `103`. A blocked mutation is reported before anything is read or retained: the guard refuses with `[104, 0, 0, 0, 0]`, so the reply carries the code alone. The key that blocks it is read separately, from word 4 of a `TASKS_STATUS` reply, and is the version of the record that is still unresolved.

Recovery is the successor's, not the dead process's. Killing the child does not cancel an exchange it already submitted: the file service still completes it, so the target may change after the process was reaped, and a disk read taken at the moment of death proves nothing. A successor launched on the same journal must therefore ask `TASKS_STATUS` first and resolve what it reports, instead of inferring the outcome from the document.

`tasks-owner-forget PID KEY` requires the exact retained key: a key that is not the retained one is refused with `104`, the same code an intent that blocks a mutation reports, and the retained version stays readable from `TASKS_STATUS`; once nothing is retained any key reports `103`, because there is no record to discard. The refusal that ends an apply decides what the client still holds: a non-conclusive refusal keeps the candidate, so the client stays in phase 3 and the plan remains available to the recovery that must resolve it, while a conclusive refusal or a success concludes the attempt and moves it to phase 4.

Host tests in `apps/utility` cover the step machine and the reply encoding: which step each phase accepts, that out-of-order bytes never become a candidate, that a new edit or begin replaces whatever was held, and that a proven effect is still reported when the operation failed. Guest evidence comes from two suites that drive the real child through the shell: the standalone `python3 tools/tasks_owner_test.py` (a disposable two-boot volume) and the `tasks.owner` phase of `python3 tools/terminal_test.py`. Both exercise add/done parity with the shell client against the same committed bytes, an unchanged edit that submits nothing, a human version conflict, a revoked grant, retention exhaustion as `Full`, out-of-phase steps, the launch refusals — including a second scope the service withdraws, after which the slot is immediately reusable — and process death followed by the successor's query, recovery and forget. The standalone suite adds a second boot: the grant, the record location and the protocol survive an actual reboot and the child commits an edit after it. The evidence is `artifacts/tasks-owner-test/tasks-owner.json` (including its `result.after_reboot`) and the `tasks.owner` object of `artifacts/terminal-test/terminal.json`.

## Remaining work and acceptance

General capability discovery, independent application installation and the complete mission acceptance remain open in [#22](https://github.com/alseif0x/rustic-os/issues/22). The second semantic client above shares the owner-client mechanism through `rustic-tasks-client` and is exercised in the guest, but it is still launched from the same shell-driven supervisor and speaks the same task document; it does not establish general discovery. MCP is optional. The initial two-outcome limit is real: a third tracked write reports `Full` until the owner explicitly changes retention. Workspace scaling remains [#51](https://github.com/alseif0x/rustic-os/issues/51); task commands introduce no new file-service profile or kernel mechanism.

`python3 tools/tasks_write_test.py` runs a separate disposable two-boot volume so mutation receipts cannot disturb the terminal's existing recovery evidence. It exercises add/done/no-op, full candidate transport, capacity refusal, policy denial, a human version conflict, prepared-but-unsubmitted recovery, lost journal and effect responses, restart/reboot, an actual older numeric-key collision, later human edits and expired outcomes. It uses real guest commands and an independent disk reader. The failure cuts are available only in explicit acceptance builds. Normal builds omit them.

The same cuts are available to the second client, so its ambiguous outcomes need no race. An acceptance build accepts `tasks-owner-apply-cut PID CUT`, which the supervisor translates to the ordinary apply action carrying the selector: `0` none, `1` retain the intent and stop before submitting it, `2` submit the replacement and discard its reply, `3` retain the intent but lose the acknowledgement of the retention, `4` write foreign bytes over the target first so the one submission is refused by version. The request identifier `TASKS_OWNER_APPLY_CUT` (35) is reserved in every build, but only an acceptance build translates it; elsewhere the supervisor refuses it as invalid, the shell does not have the command at all, and the utility refuses a non-zero selector as an out-of-phase step (`105`) without touching a file. Cuts `1` and `3` leave the record retained and unresolved, which is what a successor's `TASKS_STATUS`, `TASKS_RECOVER` and `tasks-owner-forget` are for. Cuts `2` and `3` also finish the child that took them: discarding the reply unbinds its file client, and unlike the shell it holds no supervisor authority to rebind, so every later step of that child answers `31` (unavailable). Resolution always comes from a new tasks-owner child launched on the same document and journal, which is the situation the cuts exist to reproduce.

The native terminal acceptance covers ordinary/empty/maximal lists, malformed documents, duplicate IDs, capacity errors, policy denial, occupied process slots, repeated resource reuse and a second boot. It compares the committed disk bytes before and after each query using an independent host reader. Pure Rust tests cover parsing and wire validation.

## Native lifecycle acceptance

`python3 tools/terminal_test.py` explicitly enables the `tasks-acceptance` build feature in the shell, the supervisor and the utility. Ordinary application builds and the interactive terminal omit it. The source build ID includes that feature profile, and the evidence records the kernel hash and profile. No kernel or file-service test opcode is added.

The host invokes a deterministic client inside the existing shell, using its authenticated supervisor connection. Three bounded cases exercise the real task process and service exchange:

- Cancel after the permission RPC was sent, while its response is held before consumption. Require the real response to drain, reject the stale result and successfully run a fresh listing. This does not claim cancellation happened before the file service applied the grant.
- Hold a task in its reserved process slot, launch an ordinary `spin` utility concurrently, then finish the task. Require distinct PIDs and continued supervisor ownership of the utility before killing and reaping it.
- Retrieve one result row and abandon the remaining result. Wait without sending owner requests, then require the recorded cleanup tick to precede the next owner query. A cleanup triggered only by that query must fail acceptance.

The fixture checks exact task rows and restored frame/process/channel/I/O counters. An independent host reader also requires unchanged committed disk bytes. The three structured records live under `tasks.lifecycle` in `artifacts/terminal-test/terminal.json`; missing or contradictory records fail the suite. Instrumentation only holds a real pending exchange or records a real cleanup transition; it does not fabricate grants, task rows or resource release.
