<!-- SPDX-License-Identifier: Apache-2.0 -->

# R0 process ABI — version 1.0

Minimal #10 contract, preserved by the [#34 IPC/handle extension](IPC.md). The SDK remains in #11. The shared source of constants is `crates/abi`, with no kernel dependency.
This is neither the Linux ABI nor an MCP interface. Identity comes from the process the kernel is executing; no argument can replace it.

A static x86_64 ELF64 program enters at e_entry in ring 3, with CS=0x2b,
SS=0x33, RSP aligned to 16 bytes, IF=1 and IOPL=0. It receives no return
address and must terminate through EXIT. The initial implementation supports
integer code without x87/MMX/SIMD, TLS or dynamic linking; the kernel does
not rely on a red zone. RDI, RSI and RDX contain three startup integers supplied
by the test launcher. Other general-purpose registers start at zero.
The user stack is 16 KiB RW/NX with an unmapped page below it.
SYSCALL/SYSENTER are not supported entry points: the kernel disables these paths.

INT 0x80 uses RAX as the call number and RDI as the integer argument. It returns
a 64-bit integer in RAX and preserves other general-purpose registers.
RFLAGS preserves arithmetic flags and DF; the kernel sets IF and removes
unsupported control flags before resuming. These calls contain no user
pointers or buffers and do not modify caller memory.

| RAX | Name | RDI | Result |
| --- | --- | --- | --- |
| 0 | QUERY | Ignored | 0x00010000, version 1.0 |
| 1 | EXIT | u64 exit code | Terminates; does not return |
| 2 | REPORT | u64 diagnostic value | 0; at most eight values per process, then QUOTA |
| 3 | GET_PID | Ignored | Monotonic identity assigned by the kernel |

Unknown numbers return NOT_SUPPORTED = u64::MAX. QUOTA = u64::MAX - 1.
REPORT retains a counter and last value in the process record: it grants
no arbitrary I/O and accepts no user text to interpret as a kernel log.
Waiting for processes, termination by the test supervisor and creation are
provided through a typed internal API; they are not yet syscalls. Waiting
for a live process returns pending; waiting for an exited process reclaims
its resources and consumes its result. An unknown or already reaped PID
produces an error. The [IPC](IPC.md) extension adds owner-bound handles and
attenuable rights. Grants/transfers and cancellation by PID remain in the
trusted launcher API, without exposing arbitrary authority through a syscall.

The [native SDK](SDK.md) provides guest-only wrappers and a Rust entry macro for this ABI. It preserves the three integer bootstrap arguments and checks process/IPC versions before calling application code. Its manifest does not change authority or add syscalls.
