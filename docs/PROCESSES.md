<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native processes and isolation — #10

## Outcome and scope

RusticOS loads static ELF64 images into owned roots and executes their instructions
in ring 3. A timer returns control to the kernel even when a program requests no
services. The scheduler selects ready processes in round-robin order; an application
exception records a fault result and allows peers to continue. Creation, inspection,
stepping, termination and waiting/reaping are typed internal operations, independent
of a shell, filesystem, network or model.

This is a single-CPU implementation with at most eight resident processes.
Each program has up to 256 data/code/stack pages, including sixteen stack pages (64 KiB).
Page tables and process metadata are accounted for separately. Exited processes
occupy their slot and memory until the owner reaps the result; the bounded table
prevents unlimited zombies. Identities are monotonic during the manager's lifetime;
failed creation may consume an identity, which is never reused. Counter exhaustion
is an error.

There are no threads, fork, dynamic linking, TLS, signals, priorities, SMP,
demand paging or per-process floating-point/SIMD state. Instructions in the last
category fault the process rather than accidentally sharing extended registers.
The initial integer-call protocol is in [PROCESS-ABI.md](PROCESS-ABI.md);
[IPC, handles and buffer copying](IPC.md) are implemented. The [native runtime](NATIVE-RUNTIME.md) adds dormant provisioning and authenticated supervisor calls; broader service authority remains in #13. The GUI and agents will consume later services.

## Separation of responsibilities

| Module | Contract |
| --- | --- |
| Library `process/elf.rs` | Complete decoding/validation without unsafe, CPU access or allocation |
| `process/lifecycle.rs` | Identity, states, capacity, scheduling and result consumption |
| `crates/abi` and `process/abi.rs` reexport | Shared contracts without implementation dependencies |
| `process/runtime/error.rs` | Load/process-boundary errors without a manager dependency |
| `process/runtime/loader.rs` | Copy segments, zero-fill and roll back incomplete loads |
| `process/runtime/manager.rs` / `record.rs` | Own spaces/contexts, run one turn and apply the event result |
| `arch/x86_64/memory/user.rs` | Mappings, initial copy, bounded execution with root switch/restoration and destruction |
| `interrupts/frame.rs` | Exact register layout and user-return validation |
| `interrupts/user_cpu.rs` | Disable privilege entry paths outside the ABI and configure CPU options |
| `interrupts/user.rs` | Bounded ring 0/3 transition, event capture and caller restoration |
| `interrupts/segments.rs` / `table.rs` | Segments, TSS, entry stack and DPL 3 call gate |
| `process/runtime/tests/` / `fixture.S` | Original applications and guest scenarios, separate from mechanisms |
| `tools/boot_support/process_evidence.py` | Reject incomplete success, incorrect faults and resource loss |

The manager receives the memory owner for operations; it does not publish it
globally or lend it to IRQ handling. It must terminate/reap all its processes
before being abandoned. The memory owner and upper mappings stay alive throughout
execution. The API is still internal to the test binary.

## Loaded format and provenance

The adopted format is little-endian ELF64, ET_EXEC, EM_X86_64, System V ABI
identification version zero and ELF version one. Only PT_NULL and PT_LOAD are
accepted. Interpreters, dynamic segments, TLS and unsupported headers are rejected
rather than ignoring program requirements. Fixed header sizes are 64 and 56 bytes.
Limits are 16 headers, eight loadable segments and a 1 MiB file.

Sums are checked before indexing or allocation. Loadable segments are ordered
and share no pages; unaligned starts are accepted when offset and address are
congruent. Validation checks power-of-two alignment, file bounds, filesz <= memsz,
budget, and exclusion of the null page, upper half and stack reservation. Only
R, RX and RW are accepted; W+X is invalid. The entry point belongs to file-backed
bytes of an executable segment. Debug sections are ignored: the loader uses
program headers. This is an explicit subset of the [ELF format](https://gabi.xinuos.com/elf/02-eheader.html)
and its [loading contract](https://gabi.xinuos.com/elf/07-pheader.html).

The two initial applications are assembled from `fixture.S` as immutable ELF
files embedded in the boot image. The kernel validates their bytes, allocates
independent pages and copies code; it does not call their labels as ring 0
functions. They contain original native instructions and no third-party runtime.
The [native SDK](SDK.md) now builds separate Rust IPC and [block-access](BLOCK-ACCESS.md) probes through the same loader. The loader receives a byte slice and knows nothing about the embedded location; external boot modules or files will provide a later source.

Every page is cleared before content is loaded. Only the file interval belonging
to that page is copied, preserving zeroes in BSS and at edges. An error frees all
segments/tables/root acquired by the load and the reserved slot. The kernel does
not execute a partly constructed image.

## Privileges, interrupts and unsafe ownership

The GDT retains kernel code/data and adds DPL 3 code/data. TSS RSP0 points to a
16 KiB entry stack, shared by the single CPU, with a guard page. The double-fault
IST stack retains its own guard and reservation. Both guards are also unmapped
in HHDM. TSS, stacks and tables are supervisor-only; the I/O permission bitmap
lies outside the TSS limit and denies user port access. IOPL stays zero.
TSS, gates and IRET mechanisms follow the [Intel SDM](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).

Gate 0x80 is the only user-invocable gate; 0x81 is an internal DPL 0 entry
that saves the caller continuation. IRQs/exceptions save all general-purpose
registers. IRQ handling first acknowledges the PIC and updates the clock,
then copies the user frame into the bounded exchange and returns to the kernel
caller. The manager handles that event outside the handler and selects another
turn. Each tick ends the turn; a call is also a scheduling point. Kernel code
is not preempted and there are no suspended kernel stacks per process.

SYSCALL is disabled through EFER.SCE and SYSENTER MSRs are cleared when CPUID
advertises SEP. Privileged loader entry points are not inherited.
CR4.FSGSBASE/PCE are also cleared; the initial ABI offers neither TLS nor
performance counters.

The architecture dependency goes from memory to the CPU bridge; the interrupt
module imports neither memory nor processes. The loader and manager share error
types without importing one another.

The bridge's sole temporary pointer is installed with IF=0 on a live object
in the common stack. The caller is suspended while the handler borrows it;
the pointer is removed before access to the object is regained. No allocations
or references to lower memory survive a CR3 switch. NMI, double fault and machine
check do not borrow that exchange: they remain terminal kernel events.
A ring 0 fault is not attributed to an application to conceal it.

Selectors and instruction/stack addresses are validated before return and flags
are filtered. A noncanonical RSP terminates the process before IRET is attempted.
CR0.TS prevents using unsaved FP/SIMD state; the caller's value is restored on
kernel return. Exception results retain vector/error and CR2 only for page faults.
The kernel restores its root before freeing resources.

This isolation covers the accesses and instructions tested in R0. It does not
establish resistance to side channels, driver/kernel bugs, different hardware
or a malicious loader. There is no independent audit; review is by the implementer.

## Reproducible tests

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 15
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

The `ok` scenario requires process evidence in addition to IRQ and memory:

- A noncooperative program advances in a loop without calls, receives at least
  two timer preemptions and allows a peer to finish. Both use the same virtual
  data address with distinct values.
- Reads/writes to a peer-exclusive page and the kernel, code writes, stack
  execution and guard accesses produce the specific page fault. CLI, invalid
  opcode, unsupported FP, kernel entry and invalid return are also contained.
  After each case, the peer receives another CPU turn.
- Disabled SYSCALL, port I/O and user HLT are rejected without allowing the
  application to disable or halt scheduling.
- Programs check CS/SS, stack alignment, BSS, initialized data, identity, version,
  unknown call, preserved values and diagnostic quota.
- Checks cover full capacity, pending wait, explicit termination, a result
  reaped once, new identities and 16 complete cycles without frame loss.
- The guest rejects invalid executables before allocation. Temporarily reserving
  the real inventory until 0, 5 or 9 pages remain forces failures during root
  creation or partial segment/stack loading. The exact free counter is restored.

At #10 acceptance the suites had 13 VM and 17 executor scenarios: process acceptance and
contained negative cases are integrated into `ok`. Kernel faults in other
scenarios remain expected terminal events. Validation/policy contracts also
have host tests; those tests do not replace ring 3 execution.

Closing evidence links the commit, CI, versions and artifacts in #10.
Configuration: one-CPU, 256 MiB R0, QEMU 8.2.2 q35/qemu64/TCG, OVMF 2024.02 and
Rust 1.98.1. Diagnostics record preemptions, contained faults, repetitions and
free memory before/after. Timings include self-tests and are not product-boot
latency. The future integrated scenario's 2 GiB budget still requires raising
the #9 physical limit; this test does not establish it.

`PROCESS_MEMORY` records maximum frames used when all eight slots are filled,
including page tables and user stacks, the actual `Manager` size and the
additional 20 KiB entry-stack reservation (including guard). This static
reservation is not charged again per process. Each fixture ELF occupies
8,200 bytes in the image; two variants are embedded as test data, outside
dynamic root consumption. The three exhaustion points are recorded separately.

#34 adds the Blocked state, syscall modules and IPC integration. Handles close
when the process exits, even if its pages remain until reaping. Wakeup policy
stays outside the handler. Manager-size diagnostics now include the broker
and wait records.
