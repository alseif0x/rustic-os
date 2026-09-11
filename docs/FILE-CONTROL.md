<!-- SPDX-License-Identifier: Apache-2.0 -->

# Owner control during logical file replacement

This increment implements the next storage/control boundary for #12/#13/#43. The native `files.replace` completed-operation profile receives private owner revocation while an admitted block command is pending. The same service policy and filesystem publication state machine run in host tests and in the native file-server process. This is not durable asynchronous operation admission or a public `operations.cancel` endpoint.

## Ownership and scheduling

`rustic-fs::PollDisk` defines nonblocking write/flush completion with one owned command. The native adapter retains the kernel request ID, operation and copied bytes, and checks the matching completion before proceeding. A changed command, malformed completion or I/O error fences the adapter. It never calls the blocking block wait from this path. Kernel block ownership, device deadlines and DMA reclamation are unchanged.

The service owns volatile grants, roots and staging in `Clients`, separately from its `Volume`. `Server::commit_with` exclusively borrows storage while a bounded callback can revoke, detach or expire clients. New grants require the full server and volume. Scope, subject and the filesystem ancestry cannot change under that storage borrow; the service checks the live peer, generation, rights and expiration before every poll and before releasing a receipt. Revoking a parent also fences its helper. Revoking an unrelated root does not cancel the active operation.

The native callback polls the authenticated private administrator endpoint. Revocation, root revocation, detach and status remain available. Other administration returns `Busy` during a logical commit. Ordinary client requests remain queued within existing IPC limits until the commit completes. Existing pending replies keep their original endpoint and correlation; revocation clears undelivered replies to the affected contexts. There is no quota expansion or extra resident process.

When storage returns `Pending`, the loop waits on administrator IPC for at most one guest tick before querying the device again. The current wait set does not combine block completion and IPC notification. This bounded polling is sufficient for the reference platform; it is not a general asynchronous event loop or a throughput claim.

## Revocation and effect evidence

Receiving revocation immediately fences future admission. Its acknowledgement is deferred until the active publication settles or becomes uncertain. While the device completion is withheld, the owner continues to see unknown effects. A request/observation deadline is not proof of settlement.

Before header submission, revocation stops the writer after any admitted scratch command settles. The original file bytes/version remain selected. The client receives `Revoked` or `Expired`, with no speculative receipt. If the command fails while draining, the result is `Uncertain` and explicit service recovery is required.

Once the header may have been submitted, cancellation is too late. The writer finishes the final flush, but does not release the receipt under revoked authority. The client receives `Uncertain`; a fresh authorized lookup can retrieve the committed original receipt. The private acknowledgement's `effects=settled` means no unresolved I/O remains in this service, not that an earlier write was rolled back. The file/version/receipt is the independent effect evidence. Revocation does not create a durable cancelled operation record.

If the administrator channel closes, the callback detaches clients and drains the publication before the service exits. Process termination outside that loop continues to rely on the kernel's existing abandoned-request ownership and subsequent remount.

## Validation and implementation review

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode recovery-test --timeout 60
```

Host tests with withheld completions revoke at all 17 admitted-command positions, exercise repeated pending control, failure while draining, expiry, detachment, invalid peers, helper/root isolation, drop/forget and adapter unwind. Existing synchronous tests continue through the same publication path. These models do not establish native scheduling or physical power-loss behavior.

The native recovery fixture adds four cases with two VMs each: held data write, held flush immediately before the header, held header write and held final flush. It uses the real native utility, file-service IPC and VirtIO driver. The kernel diagnostic retains completion observation for an already submitted command; it does not simulate a device failure. The owner revokes the utility while `pending_io=1`, observes unknown effects during the hold, remains responsive, and receives settlement only afterward. Early cases preserve the old file; late cases preserve the committed receipt. Reboot lookup and an independent CRC/content/version/receipt oracle verify each result, including unchanged unrelated files and reclaimed resources. The recovery inventory is now 19 groups / 38 VM boots; the full direct/isolated inventories remain 22 / 26 scenarios.

During development, real execution caught an increased cumulative stack footprint in the legacy tracked-write path. A QEMU CPU trace identified a ring-3 page fault at `0x7ffef408`, below the 64 KiB application stack. Keeping the new commit controller out of unrelated dispatch frames with an explicit non-inline boundary fixed that regression without enlarging the stack or weakening its guard. Native regression execution, rather than host compilation alone, validates this boundary.

Review covers separate storage/authority/transport ownership, directed crate dependencies, terminal receipt visibility and outstanding-command lifetimes. This control increment introduced no new unsafe code, external dependency, kernel policy, wire opcode or on-disk format. Ordinary reads, legacy file mutations and storage administration still use synchronous I/O. The subsequent [admission storage increment](FILE-ADMISSION.md) adds persistent identities and retained cancellation in an explicit format upgrade. The [typed service integration](FILE-ADMISSION-CONTROL.md) now polls those transitions under owner control and fresh execution authority. Its public IPC binding, delegated cancellation, current-operation discovery and full native/function/MCP conformance remain subsequent work.
