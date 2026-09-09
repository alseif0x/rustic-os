<!-- SPDX-License-Identifier: Apache-2.0 -->

# Physical memory, virtual memory and protections — #9

## R0 scope

The kernel allocates and frees 4 KiB physical frames, builds owned page tables and switches between address spaces with independent data. The baseline remains x86_64, four-level paging, one CPU and QEMU q35/TCG with 256 MiB. LA57, PCID and CPUs without NX are rejected. No new Cargo dependencies or toolchain change are introduced.

The minimal allocator operates at page granularity. There is no `GlobalAlloc`, `Box`, variable-size heap, swapping or NUMA yet. This subsystem's own tests switch CR3 in ring 0; #10 separately adds [processes and scheduling tested in ring 3](PROCESSES.md).

## Responsibilities and ownership

| Module | Responsibility |
| --- | --- |
| Library `memory/frames.rs` | Inventory, reservations, bitmap allocation and release, without pointers or CPU access |
| Library `memory/page.rs` | Valid virtual addresses and permission policy |
| `arch/x86_64/memory/physical.rs` | Sole owner of bitmaps, frames and raw HHDM access |
| `cpu.rs` | CR0.WP, EFER.NXE, CR3 and translation invalidation |
| `tables.rs` | Table walking, effective permissions and splitting large leaves |
| `bootstrap.rs` | Private copy of loader tables and kernel/alias protection |
| `space.rs` | Root lifecycle, mapping, unmapping and resource return |
| Architecture `memory/tests/` | Allocation, exhaustion, spaces, large leaves and deliberate faults |
| `boot/limine.rs` | Translates loader responses into owned values; no Limine types leak into the allocator |

`Memory` owns the physical allocator and kernel root; secondary spaces require that root and its upper tables to stay alive. The token is CPU-local. IRQ/NMI handlers do not allocate or free memory, and methods require exclusive access to the owner. This model does not replace locks/SMP or authorize concurrent interrupt allocation. No Rust references are exposed to pages that address-space switches could invalidate.

## Inventory and reservations

Two static 32 KiB bitmaps track manageable and allocated frames: 64 KiB of metadata for physical addresses below 1 GiB. This is an explicit implementation limit, not a claim of tested support for a 1 GiB machine. A usable region exceeding it is rejected before import, not silently ignored. Raising it requires reviewing metadata, addresses, budgets and tests.

The entire map is validated before pages are imported. Only whole pages from `USABLE` regions are included; unknown types, firmware, ACPI, executable/modules and loader memory are excluded. The first MiB and the full ELF physical extent are also reserved, using the executable-address response and linker symbols. Reservations round outward; usable regions round inward. Double frees, unaligned addresses, unmanaged frames and reservation of in-use frames are rejected.

The loader remains trusted. Its tables, responses and boot stack stay reserved even after owned tables exist. `USABLE`, HHDM and executable physical-address semantics follow the [pinned Limine protocol revision](https://github.com/limine-bootloader/limine-protocol/blob/da65184e91f80fcb397270121b1e2515a11e01ee/PROTOCOL.md). The R0 validator requires a sorted, nonoverlapping map, including reserved regions: it may reject maps another platform could accept; universal compatibility is not established.

Inventory exhaustion returns a typed error and preserves state. A whole page is cleared before exposure through another mapping or space. The allocator does not search indefinitely, steal reserved memory or substitute a null address for failure.

## Tables and permissions

Before activating an owned root, the loader's upper tables are recursively copied into new frames. The lower half is cleared, user access is removed from all inherited mappings and execution is disabled by default. Upper addresses needed for code, stack, IRQ, TSS and HHDM are retained. The root and all descendants are owned; loader tables are not modified.

Linker symbols delimit code, read-only data, writable data and the ELF end. Code becomes RO/X; constants and requests RO/NX; writable data, bitmaps and tables RW/NX. HHDM aliases of code and constants are also read-only and nonexecutable. Without alias protection, another address could modify the same physical code. The #33 emergency stack retains 16 KiB usable space and gains a guard page, unmapped at both its kernel address and HHDM alias. The inherited boot stack does not gain a guard in this increment.

Where 4 KiB granularity is needed, inherited large leaves are split while preserving addresses and cache attributes, including PAT's different bit position. The 1 GiB case is checked structurally through a synthetic root that is never activated; execution of that leaf on the reference CPU is not established. Boot and real accesses verify the mappings R0 uses.

CR0.WP and EFER.NXE are enabled and CPUID is checked for NX. PGE remains disabled and new tables create no global entries. CR3 switching and translation invalidation follow volume 3 of the [Intel SDM](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html). Raw table accesses are bounded to resident pages and aligned entries; operations create no Rust references that could alias hardware A/D updates.

## Virtual allocation and spaces

New mappings are restricted to aligned pages in the lower canonical half, excluding the null page. The API allocates its own frames; callers cannot choose a kernel frame. It rejects duplicate mappings and simultaneously writable/executable permissions. User access is granted only on these new pages, with effective permissions calculated through all levels.

An allocation that exhausts memory while creating tables removes added entries and returns every frame acquired by that operation. Unmapping invalidates the active translation before frame reuse, removes empty tables and reloads the root to invalidate paging-structure caches as well. No other CPUs need a TLB shootdown; introducing them requires another synchronization policy.

Secondary spaces share the kernel's upper tables and own their root, lower tables and data pages. Upper tables remain fixed after bootstrap; no API is provided to mutate them afterward. Destroying an inactive space returns its lower resources and root, never shared upper tables. Destroying the active space or kernel root is rejected. Destruction is explicit; #10 connects it to reaping an exited process. Reference counting, shared pages, copy-on-write and collection of abandoned spaces are not implemented.

If initial table construction fails, boot ends with diagnostics before the new root is activated. That terminal path does not attempt to continue as a partly initialized kernel. Rollback and resource recovery are checked for ordinary subsequent operations.

## Tests and evidence

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py test --timeout 15
python3 tools/sandbox.py prepare
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

`ok` requires IRQ and memory tests to finish before SUCCESS. Memory tests run 16 map/write/unmap cycles, reject duplicates and W+X, check code/rodata/aliases/guard, create two spaces with the same virtual address and different frames, alternate CR3 and verify their data, reject destruction of the active space and return resources. A structural test checks 1 GiB → 2 MiB → 4 KiB splitting and PAT.

Next, the guest's real physical inventory is exhausted. The temporary frame list is stored within those frames to avoid a large additional heap or stack allocation. A single frame is freed, reacquired and checked for 4 KiB of zeroes. With only two free frames left, an allocation needing four must fail and return its partial acquisitions. Finally, the exact initial free counter is recovered.

Five fixtures must produce #PF (vector 14), CR2 matching the announced address and the specific error code:

| Mode | Deliberate access | Error |
| --- | --- | --- |
| `memory-ro` | Write an RO page from ring 0 | 0x3 |
| `memory-nx` | Execute an NX page | 0x11 |
| `memory-unmapped` | Read after unmapping and release | 0x0 |
| `memory-text-alias` | Write code through its HHDM alias | 0x3 |
| `memory-guard` | Read the emergency-stack guard | 0x0 |

Explicit test instructions are used without forming invalid Rust references. An incorrectly allowed access reaches UD2 and does not satisfy the expected result. A #DF, different address/code or timeout also fails the protection test. These are terminal test-VM faults; survival of a second process after an application fault is checked separately in [#10](PROCESSES.md).

At #9 acceptance the direct suite had 13 scenarios and the isolated suite 17, preserving previous #8/#21/#33 cases. Results and logs remain in the same evidence directories; each image retains revision, configuration and hashes. An initial 256 MiB sample recorded 52,795 managed frames, 15 owned-table frames and 52,780 free before and after exhaustion, with 65,536 bytes of metadata. Full boot including self-tests took around 6.3 seconds. These are measurements of that revision/R0, not universal thresholds. The issue links the closing commit and CI; review was by the implementing agent, without independent review.
