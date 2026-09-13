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

Each mutation first collects the exact candidate bytes from the native planner and checks the original version again. The owner client reserves `/config/tasks-intent` for one immutable recovery record: volume lineage, selected workspace/resource, original version, retry epoch, command, affected ID and candidate bytes/hash. The record fits within the existing 1,024-byte file bound. Its atomic legacy replacement does not consume a tracked receipt. An exact version-pinned readback must succeed before any target submission.

The journal's committed metadata version becomes the initial retry key under the existing stable owner subject `1`. The file effect uses the existing completed-operation profile. A matching receipt must have a version greater than that journal version; an older diagnostic operation with the same numeric key cannot establish success. The client then independently reads the pinned committed version and compares the exact bytes before clearing the intent with a version-checked replacement. The idle journal remains as one empty file. Already-done commands make no writes and consume no receipt.

One unresolved intent blocks new task mutations. `tasks recover` queries the original retry tuple under the current owner binding and performs the same receipt/byte checks. It never resubmits or rebuilds the edit. After a disconnected file binding, run `restart files` first; that existing command revokes utility sessions. Missing or expired outcomes remain uncertain. If a human changed the target after the recorded commitment, the client reports the historical commitment but retains the intent because current bytes cannot be verified at that version.

Losing the journal commit response stops before target submission. After restart, the record may exist, but an absent outcome does not establish which instruction the earlier process reached. Recovery remains query-only. `tasks forget INTENT_KEY` deliberately discards that recovery record; it does not cancel or undo a submitted effect. The key must match the currently retained journal version. Corrupt records require explicit owner repair, not automatic interpretation.

The journal is owner-controlled application state, not a new kernel authority or tamper-proof ledger. Do not edit, delete or reuse it while an intent is unresolved. The identity guarantee follows surviving committed history of one volume lineage; cloning or rolling back an entire volume is outside this single-owner protocol. Other owner diagnostics share the two retained outcome slots and numeric key namespace. Collisions and expired epochs stay explicit; task commands neither rotate receipts nor silently expand capacity.

## Remaining work and acceptance

General capability discovery, independent application installation, a separately implemented semantic client and the complete mission acceptance remain open in [#22](https://github.com/alseif0x/rustic-os/issues/22). MCP is optional. The initial two-outcome limit is real: a third tracked write reports `Full` until the owner explicitly changes retention. Workspace scaling remains [#51](https://github.com/alseif0x/rustic-os/issues/51); task commands introduce no new file-service profile or kernel mechanism.

`python3 tools/tasks_write_test.py` runs a separate disposable two-boot volume so mutation receipts cannot disturb the terminal's existing recovery evidence. It exercises add/done/no-op, full candidate transport, capacity refusal, policy denial, a human version conflict, prepared-but-unsubmitted recovery, lost journal and effect responses, restart/reboot, an actual older numeric-key collision, later human edits and expired outcomes. It uses real guest commands and an independent disk reader. The failure cuts are available only in explicit acceptance builds. Normal builds omit them.

The native terminal acceptance covers ordinary/empty/maximal lists, malformed documents, duplicate IDs, capacity errors, policy denial, occupied process slots, repeated resource reuse and a second boot. It compares the committed disk bytes before and after each query using an independent host reader. Pure Rust tests cover parsing and wire validation.

## Native lifecycle acceptance

`python3 tools/terminal_test.py` explicitly enables the `tasks-acceptance` build feature in the shell and supervisor. Ordinary application builds and the interactive terminal omit it. The source build ID includes that feature profile, and the evidence records the kernel hash and profile. No kernel or file-service test opcode is added.

The host invokes a deterministic client inside the existing shell, using its authenticated supervisor connection. Three bounded cases exercise the real task process and service exchange:

- Cancel after the permission RPC was sent, while its response is held before consumption. Require the real response to drain, reject the stale result and successfully run a fresh listing. This does not claim cancellation happened before the file service applied the grant.
- Hold a task in its reserved process slot, launch an ordinary `spin` utility concurrently, then finish the task. Require distinct PIDs and continued supervisor ownership of the utility before killing and reaping it.
- Retrieve one result row and abandon the remaining result. Wait without sending owner requests, then require the recorded cleanup tick to precede the next owner query. A cleanup triggered only by that query must fail acceptance.

The fixture checks exact task rows and restored frame/process/channel/I/O counters. An independent host reader also requires unchanged committed disk bytes. The three structured records live under `tasks.lifecycle` in `artifacts/terminal-test/terminal.json`; missing or contradictory records fail the suite. Instrumentation only holds a real pending exchange or records a real cleanup transition; it does not fabricate grants, task rows or resource release.
