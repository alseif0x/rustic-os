<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native session runtime extension 1

Implemented bounded runtime for #45 and the first #13/#14 terminal increment. Base process ABI 1.0, IPC 1 and block 1 remain unchanged. No caller identity argument grants authority.

## Capacity and execution

Eight process slots support supervisor, file server, shell and two utilities, with three bounded spare slots for recovery/negative fixtures. Eight IPC channels provide private supervisor-shell and supervisor-files control plus file-client channels; 32 handles, 16 per owner, retain two 64-byte messages per direction. User stacks are 64 KiB with an unmapped guard, allowing bounded service buffers/nested codecs while retaining the 256-page process limit. #20 receives measured occupancy and pressure results; these are implementation bounds, not general performance claims.

Newly spawned programs remain dormant until the authorized supervisor sets bootstrap arguments and starts them. Names/manifests are descriptive; the executable catalog is explicit trusted native ELF data. No host path or host command is accepted. File format/session policy stays in native services; the kernel supplies lifecycle, owned handles, memory and device mechanisms.

## Additive syscall convention

INT 0x80 keeps RAX as number/result and RDI/RSI/RDX as arguments. No Rust layout crosses the boundary.

| Number | Operation and bounded arguments | Result |
| --- | --- | --- |
| 15 | Clock/yield: zero arguments | Monotonic PIT ticks; scheduler resumes another ready process |
| 16 | Console write: pointer, 1–256 bytes, 0 | Written byte count; current console owner only |
| 17 | Console read: pointer, 1–64 bytes, 0 | Copied bytes or WouldBlock; validate full output before consuming input |
| 18 | Console wait: zero arguments | Block until input is available; retain no pointer |
| 19 | Wait set: pointer to 1–8 u64 IPC tokens, count, timeout ticks 0–1000 | 0 on readiness/timeout; invalid/closed handles wake with a typed error; pointers are copied before blocking |
| 20 | Supervisor control: in/out pointer, exactly 64 bytes, 0 | 64 bytes; current trusted supervisor only, full write range validated before effect |

The control packet is eight little-endian u64 words. Word 0 is the opcode. INFO=0 returns runtime version/ticks/free frames/capacity/occupancy. SPAWN=1 takes a trusted catalog identifier and returns a dormant child PID. START=2 takes child PID and three bootstrap integers. CONNECT=3 takes two owned child/self PIDs and returns their separate handles. BLOCK_GRANT=4 takes child PID, rights, first sector and sector count. CONSOLE_GRANT=5 assigns/revokes the exclusive console child. PROCESS=6 queries a bounded slot and returns PID/state/exit/preemptions/parent/catalog entry. KILL=7 and REAP=8 address owned children only. SHUTDOWN=9 requests clean session teardown. DEVICE=11 reports current block geometry. Unused request words must be zero. Supervisor selection is trusted boot state, never a packet field.

File-service scope generations are separately bound to the authenticated client endpoint/PID. A moved token cannot create a new service grant. Console ownership is independent from file rights: child output cannot write directly to the owner's prompt. Services wait on bounded endpoint sets; when all user processes wait, the kernel sleeps until the next timer interrupt and continues polling disk/input and deadlines. No permanently spinning control process is required for an idle terminal.

MOVE_ENDPOINT=13 takes source PID, source token, target PID and attenuated IPC rights. Only the trusted supervisor may call it, both processes must be live and supervisor-owned, and it returns the new target token while invalidating the old token. File-service identity and session roots are unchanged; [native authority acceptance](AUTHORITY.md) checks movement, denial and root death.

CLOSE_ENDPOINT=12 takes an owned PID/token to roll back trusted provisioning. It cannot close a foreign process's authority. Full-capacity loader/lifecycle tests cover all eight slots; the terminal deliberately limits utilities to two.

## Accounting and limitations

The normal topology is three resident processes and four channels: supervisor–files admin, supervisor–files owner data, supervisor–shell control and shell–files data. Two scoped utilities raise it to five processes and eight channels. Each program has at most 256 data/code/stack pages, including 16 stack pages; page tables are separately accounted. Pending wait sets copy at most eight tokens. Block capacity remains two request records/four grants, with one active device operation and three DMA frames.

Runtime errors use `u64::MAX - 64 - index`: Denied, Address, Size, Invalid, Busy, NotFound, Full, WouldBlock, Closed, Protocol. Counts/tokens remain in their documented non-error ranges. PROCESS state values are empty=0, dormant=1, ready=2, running=3, blocked=4 and exited=5; exit kinds are normal=1, fault=2, killed=3. A normal exit reports its code; a fault reports its vector.

Terminal images currently use the sdk-test build feature to include the native catalog alongside acceptance probes. The non-catalog boot-image target remains checked separately. This is an explicit packaging boundary, not a claim of production image hardening. No syscall allows arbitrary executable bytes, arbitrary supervisor identity or device selection.

The boot supervisor is the trusted capability root. Services implement owner/helper rules through their private channels; manifests are admission requests only. Recovery is explicit and bounded, not an automatic restart storm. A failed supervisor stops the native session. A filesystem mount failure preserves the image and fails startup; offline repair is not implemented.
