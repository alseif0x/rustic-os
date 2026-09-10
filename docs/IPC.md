<!-- SPDX-License-Identifier: Apache-2.0 -->

# R0 IPC and handles — #34

## Binary contract

The extension uses INT 0x80; #10 numbers 0–3 and QUERY=0x00010000 are preserved.
`crates/abi` is the source of constants, independent of the kernel and SDK.
Discover this extension through INFO; SYSCALL/SYSENTER remain disabled.

| RAX | Operation | RDI | RSI | RDX | Result |
| --- | --- | --- | --- | --- | --- |
| 4 | SEND | Handle | Source pointer | Length | 0 or error |
| 5 | RECEIVE | Handle | Destination pointer | Capacity | Bytes copied or error |
| 6 | WAIT | Handle | Ignored | Ignored | 0 when a message is available; error on closure/cancellation |
| 7 | CLOSE | Handle | Ignored | Ignored | 0 or error |
| 8 | INFO | Ignored | Ignored | Ignored | IPC version: 1 |

RAX contains the result; other general-purpose registers are preserved. SEND and
RECEIVE are nonblocking. WAIT waits for receive readiness without consuming or
copying; call RECEIVE afterward. No user pointer is retained during a wait.
If all tasks are blocked, the manager reports no ready work; it does not poll
or promise to detect/resolve deadlocks.

IPC errors are `u64::MAX - n`: n=2 foreign/invalid/stale handle, 3 permission
denied, 4 invalid address, 5 invalid size, 6 incompatible version,
7 invalid message, 8 empty/full queue, 9 closed endpoint, 10 cancelled wait,
11 exhausted quota. Values n=0 and n=1 remain reserved by the previous ABI.

A message has a 24-byte header and up to 64 payload bytes. Integers are little-endian:
u16 version at 0, u16 opcode at 2, u32 payload length at 4, u64 correlation at 8
and u64 sender identity at 16. The supplied length must be exactly 24+payload;
only version 1 and opcode DATA=1 are accepted. The sender field must be zero
when sending: the kernel replaces it with the executing process's PID.
Reception returns that identity, including for messages queued before the
sender died. Rust structs, padding and kernel addresses are not transmitted.

Correlation and payload are application data; they grant no authority.
Initial negotiation queries INFO and uses version 1; a different version/opcode
is rejected without enqueueing. The #6 service contracts will be another layer.

## Ownership, rights and limits

The broker owns at most four duplex channels, with two messages per direction.
The table has a defensive limit of 16 entries and eight per owner; four channels
without duplication produce at most eight active handles. Monotonic tokens are
not reused during the broker's lifetime and are checked together with the owner.
Knowing or guessing another process's token does not allow its use.
They are not claimed to be cryptographic secrets.

READ=1 allows receive/wait, WRITE=2 allows send and TRANSFER=4 allows moving
the endpoint. Closing requires ownership, without an additional right. The
trusted launcher creates the pair and grants each endpoint to a live process.
Transfer is an internal launcher operation: it requires TRANSFER, moves the
endpoint and its pending messages, only reduces rights, invalidates the old
token and issues a new one for the recipient. It does not duplicate references.
Destination/capacity are validated before changing ownership. The launcher
provides tokens through arguments before first execution.
Creation/transfer/cancellation by PID are not exposed as syscalls without
supervisor authority; that policy belongs to #13. Including an integer in a
message does not implicitly transfer a handle.

Closing or terminating a process removes its handles immediately, even if its
memory is reaped later. The closed endpoint's queue is discarded; its peer can
consume messages already in its own queue and then receives Closed. It cannot
send new messages. Closing both endpoints reclaims the channel. Internal
cancellation wakes only the selected blocked process and completes WAIT with
Cancelled; the handle remains available for a later operation. No user-memory
alias or borrow remains pending.

## Validation and execution

The kernel checks owner/right before accessing the buffer. SEND/RECEIVE lengths
are limited to 24–88 bytes. RECEIVE does not consume if capacity is insufficient,
the destination is invalid or write permission is missing. The validated range
is the data actually copied, not additional unused capacity bytes.

Memory checks overflow, the lower canonical half, the null page, presence and
effective USER/WRITE permissions on every page before copying any byte.
Transfer uses the HHDM of owned frames: it never creates a Rust reference to a
user pointer. Source/destination belong to an inactive root and no user code
runs between validation and copying/consumption. This implementation uses one
CPU, no DMA to those pages and no page-table mutations from IRQ handlers.
SMP or shared memory will require additional pinning/synchronization;
validation followed by copying alone would not suffice.

The broker, queues, messages and handles are pure mechanisms in `kernel/src/ipc/`.
Architecture `memory/copy.rs` owns physical accesses. The syscall module translates
the ABI and errors; `ipc_control.rs` connects grants, closure and wakeups to the
lifecycle. The scheduler only adds the Blocked state and its transition to Ready.
IRQ handling still knows nothing about processes, IPC or allocation.

Completion and testing documentation is backed by guest evidence; constants or
host tests alone do not establish IPC between real processes.

## Tests and evidence

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 20
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

`ok` requires IPC acceptance in addition to memory/process/IRQ checks. Two original
ELF applications in ring 3 exchange requests/responses with verified payload,
correlation and identity for 16 cycles. Checks cover empty waits, no ready work
when all processes are blocked, launcher cancellation, user CLOSE, peer death,
draining already queued messages and reclamation.

Twenty-two negative cases invoke syscalls from user mode: null, overflowing,
noncanonical, kernel, unmapped and hole-crossing pointers; short/oversized lengths;
version/opcode, forged identity; read-only destination; foreign/stale handles
and denied rights. Checks require the expected error code and preserved queue.
An invalid receive crossing a hole must leave even the valid prefix unchanged.
Messages are also copied successfully across two distinct pages; filling the
queue checks lossless FIFO behavior. An endpoint is moved with READ rights to
a new owner, and attempts to regain removed rights are rejected. Closing the
old owner does not remove the moved endpoint.

Host tests cover format, sender authentication, FIFO/backpressure,
movement/attenuation, capacity, recovery and nonreuse of tokens, plus
Blocked/Ready transitions. The host verifier requires all markers and codes,
zero remaining resources and equal free-frame counters; a partial log or
misclassified error cannot become success.

R0 retains one CPU, 256 MiB, QEMU 8.2.2 q35/qemu64/TCG, OVMF 2024.02 and
Rust 1.98.1. At #34 acceptance these checks were integrated into 13 VM and 17 isolated scenarios; [block storage](BLOCK.md) later expands both suites.
A local sample measures 3,648 bytes for the manager with broker and states,
compared with 1,040 in #10; the four processes still use 52 frames for their
pages/tables. The first boot with IPC took around 8.9 seconds including self-tests.
These are executor-identified samples, not universal limits.
The closing issue links the commit/CI, hashes and isolated repetition.

Available review: implementing agent, without an independent audit. This does
not establish SMP, DMA to buffers, shared memory, a name service, message-attached
transfers or product authority. Internal launcher grants and cancellations are
not an open application API; #13 must give them an authority context before
exposure. Reliable IPC does not establish files, shell or an agent. The native SDK now wraps this contract.
The next increment, #11, will consume the ABI for Rust applications.

#44 reuses the owner/rights table in `kernel/src/handles/` with disjoint type domains. IPC tokens retain domain 0; block tokens use domain 1. Each call resolves the complete opaque token in its own object table and validates the current owner, preventing cross-type interpretation. IPC limits and wire version remain unchanged. Readiness for both typed waits is coordinated in `process/runtime/waiters.rs`.
