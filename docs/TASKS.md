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

## Remaining work

Only `list` is implemented. `add` and `done` need immutable mutation intent, expected-version conflict handling, retained retry identity and exact effect verification before they can be advertised. General capability discovery and the complete mission acceptance remain open in [#22](https://github.com/alseif0x/rustic-os/issues/22). MCP is an optional later adapter.

The native terminal acceptance covers ordinary/empty/maximal lists, malformed documents, duplicate IDs, capacity errors, policy denial, occupied process slots, repeated resource reuse and a second boot. It compares the committed disk bytes before and after each query using an independent host reader. Pure Rust tests cover parsing and wire validation.

Cancellation during permission provisioning, concurrent owner requests during that phase, and expiry of an abandoned result still need direct guest acceptance. Their cleanup paths have received static review; a Ctrl-C test that only interrupts shell path resolution would not establish these behaviors. These are finite follow-up checks before extending the application with mutations.
