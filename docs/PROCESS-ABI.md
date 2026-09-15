<!-- SPDX-License-Identifier: Apache-2.0 -->

# R0 process ABI — version 1.0

Minimal #10 contract, preserved by the [#34 IPC/handle extension](IPC.md) and the [#44 block extension](BLOCK-ACCESS.md). The [native SDK](SDK.md) wraps these contracts. The shared source of constants is `crates/abi`, with no kernel dependency.
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
unsupported control flags before resuming. The four base calls below contain no user
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

## Dynamic heap pages — extension 1

Three additive calls give a process pages of its own heap window at runtime.
They take three integer words: RDI, RSI and RDX, as above. Constants live in
`rustic_abi::memory`; failures reuse the `rustic_abi::runtime::Error` encoding
of the native extension, so an error is never confused with an address.

| RAX | Name | RDI | RSI | RDX | Result |
| --- | --- | --- | --- | --- | --- |
| 21 | MAP | Address or 0 | Pages | Flags | Base address of the run |
| 22 | UNMAP | Address | Pages | 0 | 0 |
| 23 | QUERY | Selector | Reserved | Reserved | Selected policy value |

`MAP` with RDI = 0 asks the kernel for the lowest free run that fits inside the
window. With an explicit address the range must be page aligned, inside the
window and completely unmapped; the kernel never moves the request elsewhere.
Flags are `READ = 0` and `WRITE = 1`; any other bit is rejected. Every page is
delivered zeroed, is user accessible and is never executable, so this call
grants no way to run new code.

`UNMAP` releases exactly one owned range and returns its frames. A range that
is not completely owned changes nothing: the release is atomic. `QUERY`
reports policy, not fixed ABI: `0` pages currently mapped by the caller, `1`
the per-process page limit, `2` the base address of the window and `3` its
size in pages. A guest reads the window instead of assuming it; the values may
change with the kernel. Selectors outside that set are invalid, and the second
and third words of `QUERY` are reserved and currently ignored.

| Error | MAP | UNMAP | QUERY |
| --- | --- | --- | --- |
| `Size` | Zero pages or more than the per-process limit | Zero pages | — |
| `Address` | Misaligned, outside the window, explicit range not free, or no run fits | Misaligned or outside the window | — |
| `Invalid` | Unknown flag bit | Third word not 0, or the range is not completely owned | Unknown selector |
| `Full` | Per-process budget exceeded, or frames exhausted | — | — |

A failed `MAP` leaves nothing mapped and nothing accounted: when frames run out
mid-run every page already mapped by that call is unmapped again. The heap
pages of a process are released with its address space when it is reaped, so a
process that exits without unmapping leaks nothing.

The [native SDK](SDK.md) provides guest-only wrappers and a Rust entry macro for this ABI. It preserves the three integer bootstrap arguments and checks process/IPC versions before calling application code. Its manifest does not change authority or add syscalls.
