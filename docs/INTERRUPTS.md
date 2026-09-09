<!-- SPDX-License-Identifier: Apache-2.0 -->

# Exceptions, interrupts and time — #33

## R0 scope and responsibilities

The kernel installs its own GDT, TSS and IDT, handles the PIT timer through the PIC and can wait for deadlines without busy-waiting. This increment uses one x86_64 CPU, QEMU pc-q35-8.2/qemu64/TCG, UEFI/OVMF and the already pinned toolchain. It introduces no Cargo dependencies or switch to nightly.

| Module | Responsibility |
| --- | --- |
| `arch/x86_64/interrupts/segments.rs` | Kernel/user GDT, TSS and 16 KiB emergency/entry stacks |
| `table.rs` / `entry.S` | IDT and register bridge between the CPU and Rust ABI |
| `dispatch.rs` | Vector classification, tick, acknowledgement and terminal diagnostics |
| `pic.rs` / `pit.rs` | PIC and PIT channel 0 ports, respectively |
| `mask.rs` | Local critical section preserving/restoring IF and waiting with STI/HLT |
| `clock.rs` | Atomic tick counter and nanosecond conversion |
| `time/deadline.rs` / `time/waits.rs` | Pure deadline and simultaneous-registration contracts; no architecture or unsafe code |
| `interrupts/tests.rs` | Guest scenarios, separate from mechanisms |
| `tools/boot_support/scenarios.py` | Evidence required by direct and isolated runners |

The entry point still composes modules. An exclusive controller token, neither Send nor Sync, owns initialization and waits; there is no manager for every subsystem. Pending registrations belong to its fixed-capacity `WaitSet`, without an allocator.

## CPU contract and memory safety

The GDT contains kernel code/data descriptors and a 104-byte TSS. The IDT has 256 interrupt gates. Vectors 0–47 have identified entries; #10 adds 0x80 (DPL 3) and 0x81 (DPL 0) for the user bridge. Others terminate as unexpected vector 255. Vector 8 uses IST1 and a separate static stack. Explicit GDT/TSS/IDT/stack storage totals 45,216 bytes after #10: two 16 KiB stacks with 4 KiB guards, seven GDT entries, TSS and IDT. This excludes alignment, code and counters. #9 adds an unmapped guard to the emergency stack and owned page tables; see [MEMORY.md](MEMORY.md).

Assembly entry normalizes the error code, preserves the 15 general-purpose registers, clears DF before calling Rust, aligns the call stack and returns through IRETQ. The frame is 176 bytes; size and critical offsets are checked at compile time. The interrupted context's registers and flags are restored on return. Gates disable maskable interrupts during the handler.

The [x86_64-unknown-none](https://doc.rust-lang.org/rustc/platform-support/x86_64-unknown-none.html) ABI uses soft-float and no red zone. Stubs do not save SIMD/FPU state; compilation with SSE/SSE2/AVX enabled is rejected. #10 revises this bridge for [ring 3, context capture and fault containment](PROCESSES.md), keeping per-process FP/SIMD disabled. Introducing SIMD, SMP or per-process FS/GS requires a new state review and tests. IDT, exception-frame, TSS/IST and IRETQ details follow volume 3 of the [Intel SDM](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html).

Tables are written through raw pointers during a single initialization with IF=0. They remain mapped for the kernel's entire lifetime; no persistent mutable references are exposed. The only later IDT mutation is the terminal fixture that deliberately invalidates the #GP gate to trigger #DF. Bootloader memory is not reclaimed.

INT3 is recoverable and increments an event counter. Other kernel exceptions terminate the test VM with vector, error, RIP, RSP, CR2 and an emergency-stack indicator; #10 separately contains supported user faults. The UART retains exclusive ownership: if an exception interrupts its owner, it neither waits for a lock nor creates an alias; serial diagnostics may be absent and the runner rejects false success. Recovery from double fault or a corrupted emergency stack is not promised.

## PIC, clock and waits

The PIC is remapped to 32–47 in 8086 mode with explicit EOI. Only IRQ0 is enabled. IRQ7/15 without an active ISR bit are treated as spurious: no EOI is sent to the controller that did not handle the interrupt; a spurious slave interrupt acknowledges only the master's cascade. Tests enter through INT 0x27/0x2f with an empty ISR: they verify this path without claiming a real electrical race. PIC/PIT registers and protocol were checked against sections 11 and 21 of the [Intel platform datasheet](https://cdrdv2-public.intel.com/332995/332995-skl-io-platform-datasheet-vol1_rev004.pdf).

PIT channel 0 uses mode 2, a nominal 1,193,182 Hz input and divisor 11,932: approximately 100 Hz, with a nominal period of 10,000,150.857 ns. The tick counter saturates and never wraps; deadlines reject overflow. Conversion computes the full ratio with 128-bit integers and saturates output without accumulating per-tick rounding.

This is monotonic time derived from handled interrupts, not UTC or calibrated wall time. Keeping IF disabled for multiple periods may lose ticks. VM pauses and host load affect observed time. Do not use this clock for TLS certificate validation or promise wall-clock bounds. APIC/HPET, SMP, suspend and wall time are outside #33.

`wait_until` requires IF enabled and a normal kernel context. It checks the deadline with IRQs disabled, then sleeps using the IRQ-indivisible `sti; hlt` sequence followed by `cli` to recover the critical section. This avoids a gap between checking and sleeping that could lose a pending interrupt. The guard restores the previous IF and supports nesting. It must not move between CPUs, be held indefinitely or be used as protection against NMI or another CPU. Handlers do not allocate, wait or acquire blocking locks; the atomic tick publishes no other data.

`WaitSet<N>` accepts simultaneous deadlines, rejects occupied or out-of-range slots, supports cancellation and delivers each completion exactly once. These are registrations handled by an owner, not suspended threads. A scheduler can consume this contract.

## Tests and acceptance

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 15
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

The original #33 increment included eight direct scenarios. `ok` requires kernel self-tests to finish before SUCCESS: INT3 preserving registers/DF, two spurious paths, nested guards, rejection of waits with IF=0, three pending deadlines (two equal), one cancellation and one hundred short waits. No completion may be early; maximum delay is two ticks in R0. Load exceeding that tolerance fails the test rather than automatically relaxing the criterion.

Additional negative cases are #UD (vector 6, no hardware error), #GP (13, error 0xfff8), a real #DF during #GP delivery (8, error 0 and `emergency=1`) and `timer-stall`: after receiving one tick, IRQ0 is masked and the wait must reach the external timeout. A triple fault, reboot, different exception or timeout before the fixture marker does not establish the expected result.

The three exception scenarios exit with QEMU 39 and status `exception`; the suite expects that deliberate failure. The individual command returns 1. `timer-stall` returns 124 at its deadline. #8 panic, hang and invalid remain. At that increment, the isolated suite totaled twelve cases, retaining #21 compilation failures, cancellation and clean repetition.

The original #33 increment had eight direct and twelve isolated scenarios; #9 expands the suites to thirteen and seventeen respectively. Artifact directories contain configuration, hashes, image/ELF, serial/QEMU logs and results. The IRQ marker adds ticks and nominal exercise duration; an initial local sample recorded 109 ticks and 1,090,016,443 ns, with total boot around five seconds including firmware and approximately one second of self-tests. It is an R0 sample, not a universal benchmark. The issue preserves the revision and CI used for closure. Review was by the implementing agent, without independent review.
