<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native tasks application

`tasks list PATH` launches a separate Rust application inside RusticOS. The shell resolves the path and presents results; the application owns the task format and validation. The supervisor grants read access to the chosen file and relays bounded results over private IPC. The application has no console authority.

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

## Remaining work

`list` and read-only edit previews are implemented. Applying `add` and `done` still needs retained request identity, expected-version conflict handling, exact effect verification and recovery after response loss. General capability discovery and the complete mission acceptance remain open in [#22](https://github.com/alseif0x/rustic-os/issues/22). MCP is an optional later adapter.

The write gate is concrete: the direct replacement SDK accepts a caller-supplied 64-bit retry key, while its historical service instance is allocated lazily at the first commit. PID, clock and transport correlation cannot provide durable request identity across restart. The existing admission API can persist the candidate and allocate an admission identity before execution, but initial-key allocation, a scoped trusted mutation subject, retained ownership across a failed child connection, and explicit profile activation must be decided before using it here. Neither a preview nor a post-commit receipt solves those pre-submit requirements. Do not hide this gap by generating fixed keys or rebuilding an uncertain candidate from newer file contents.

The native terminal acceptance covers ordinary/empty/maximal lists, malformed documents, duplicate IDs, capacity errors, policy denial, occupied process slots, repeated resource reuse and a second boot. It compares the committed disk bytes before and after each query using an independent host reader. Pure Rust tests cover parsing and wire validation.

## Native lifecycle acceptance

`python3 tools/terminal_test.py` explicitly enables the `tasks-acceptance` build feature in the shell and supervisor. Ordinary application builds and the interactive terminal omit it. The source build ID includes that feature profile, and the evidence records the kernel hash and profile. No kernel or file-service test opcode is added.

The host invokes a deterministic client inside the existing shell, using its authenticated supervisor connection. Three bounded cases exercise the real task process and service exchange:

- Cancel after the permission RPC was sent, while its response is held before consumption. Require the real response to drain, reject the stale result and successfully run a fresh listing. This does not claim cancellation happened before the file service applied the grant.
- Hold a task in its reserved process slot, launch an ordinary `spin` utility concurrently, then finish the task. Require distinct PIDs and continued supervisor ownership of the utility before killing and reaping it.
- Retrieve one result row and abandon the remaining result. Wait without sending owner requests, then require the recorded cleanup tick to precede the next owner query. A cleanup triggered only by that query must fail acceptance.

The fixture checks exact task rows and restored frame/process/channel/I/O counters. An independent host reader also requires unchanged committed disk bytes. The three structured records live under `tasks.lifecycle` in `artifacts/terminal-test/terminal.json`; missing or contradictory records fail the suite. Instrumentation only holds a real pending exchange or records a real cleanup transition; it does not fabricate grants, task rows or resource release.
