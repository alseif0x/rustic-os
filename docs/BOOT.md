<!-- SPDX-License-Identifier: Apache-2.0 -->

# Booting RusticOS

This experimental image targets the R0 virtual machine. It boots, validates data and ends the test; it is not yet an interactive session or shell. QEMU and host tools run in Ubuntu/WSL2 as the reference environment; the guest runs its own kernel.

## Run

From the repository root, inside Ubuntu with the [development tools](DEVELOPMENT.md):

```sh
source ~/.cargo/env
python3 tools/environment.py install
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode ok
```

The last command builds the static ELF and 64 MiB FAT32 volume, verifies reference packages/hashes and runs QEMU without a display, network or interactive monitor. The guest prints START, MAP and SUCCESS and exits through QEMU's test device. The command returns 0 if it verifies the marker, identity and exit status.

To build without running:

```sh
python3 tools/boot.py image --mode ok
```

The resulting image is at artifacts/boot/ok/rustic-os.img, with the ELF in the same directory. It is a removable FAT32 boot volume, without GPT, containing EFI/BOOT/BOOTX64.EFI, kernel.elf, limine.conf and notices under licenses/. The tested path uses QEMU/OVMF's virtio drive; physical hardware has not been validated.

## Cases and exit codes

```sh
python3 tools/boot.py test --timeout 30
```

| Fixture | Guest evidence | QEMU | run command |
| --- | --- | --- | --- |
| ok | SUCCESS with expected build id | 33 | 0 |
| panic | PANIC after map validation | 35 | 1 |
| hang | HANG before halting the CPU | Terminated by runner at deadline | 124 |
| invalid | FATAL for unknown mode | 37 | 1 |
| exception | #UD, vector 6 and error 0 | 39 | 1 |
| gp | #GP, vector 13 and error 0xfff8 | 39 | 1 |
| doublefault | #DF, vector 8, error 0 and emergency stack | 39 | 1 |
| timer-stall | Wait with IRQ0 masked after verifying a tick | Terminated on timeout | 124 |
| memory-ro / memory-text-alias | #PF for prohibited write, error 0x3 and expected CR2 | 39 | 1 |
| memory-nx | #PF for prohibited execution, error 0x11 and expected CR2 | 39 | 1 |
| memory-unmapped / memory-guard | #PF for absent address, error 0 and expected CR2 | 39 | 1 |
| block-persist / block-readonly / block-error / block-timeout / block-missing | Driver assertions and independent disk oracle | 33 | 0 |
| block-user / block-user-faults | Native ring 3 disk access, persistence or denials/recovery, independent disk oracle | 33 | 0 |

The isa-debug-exit device transforms the value written by the kernel into (value × 2) + 1; these codes are specific to the R0 test. Any unexpected combination returns 2. The full suite returns 0 only if all twenty cases match their expected results and markers. A firmware timeout before reaching the fixture does not pass the hang test. `ok` checks [interrupts and waits](INTERRUPTS.md), [memory](MEMORY.md), [ring 3 processes](PROCESSES.md), [IPC](IPC.md) and the [SDK](SDK.md) before SUCCESS. Separate [block scenarios](BLOCK.md) and [user-mode access scenarios](BLOCK-ACCESS.md) verify storage, caller isolation and restart persistence.

30 seconds is the initial local budget; CI uses 45 seconds to absorb runner variation. Early positive boots took around 4 seconds; the later IPC acceptance sample took 8.626 seconds including guest self-tests (see #34). These are configuration-specific observations, not universal performance targets. The host kills only the QEMU process created by that run, waits for it to exit and also removes it if the runner is interrupted.

## Evidence

Each fixture directory retains image.json (versions/configuration, commit and checkout state, source identifier and hashes), kernel.elf, rustic-os.img, serial.log, qemu.log and result.json (command, duration, timeout and status). suite.json is written only after all twenty cases finish. Logs are reset for each run; previous success is not reused.

The direct build id identifies the kernel's Rust/assembly sources, shared ABI sources/manifests and build configuration; in the isolated executor it identifies the candidate commit. It does not replace the SHA-256 hash of the ELF or image. The image contains FAT filesystem timestamps: binary identity between rebuilds is not promised. The environment is also not hermetic because of the Ubuntu baseline and its transitive dependencies.

## Responsibilities and trust

- main.rs composes entry and panic handling.
- boot/entry.rs coordinates stages; boot/limine.rs centralizes protocol requests and translation.
- boot/map.rs, mode.rs and region.rs are pure host-tested logic with no Limine dependencies.
- arch/x86_64/io.rs contains port instructions; serial.rs owns the UART through an exclusive token, bounded waits and no allocation.
- diagnostic.rs emits errors; panic does not wait for locks or alias the serial owner.
- tools/boot_support/image.py creates image files in owned directories; runner.py runs QEMU and classifies evidence.

Limine bindings 0.5.0 use base revision 3 with Limine loader 12.8.0 and a requested 64 KiB stack. The consulted [specification revision](https://github.com/limine-bootloader/limine-protocol/blob/da65184e91f80fcb397270121b1e2515a11e01ee/PROTOCOL.md) is recorded in tools/environment.toml. limine 0.6.5 was not adopted because the inspected version requires experimental ptr_metadata.

The bootloader and its pointers are trusted: the bindings rely on their validity and lifetime. Checks cover response availability, revision, nonempty/nonoverflowing ranges, ordering/nonoverlap, a 4096-entry limit and usable memory. Boot-data validation alone does not protect against a malicious loader or install owned page tables.

The kernel enters with interrupts disabled, validates boot data and installs its own GDT/TSS/IDT and PIC/PIT before enabling IRQ0, on one CPU. It then prepares [allocation and owned page tables](MEMORY.md), also protecting aliases and the emergency guard. Bootloader memory stays reserved. Testing uses a read-only boot disk, disposable OVMF variables and no personal disk or directory. Block scenarios add a separately created sparse 4 GiB test disk.

Licenses for Limine, the bindings, bitflags and Rust are included in the image. OVMF and QEMU remain external. See the [component inventory](dependencies.md). #33 adds [exceptions and time](INTERRUPTS.md), #9 [memory and protections](MEMORY.md), and #10 [native processes and isolation](PROCESSES.md). Minimal calls are documented in [PROCESS-ABI.md](PROCESS-ABI.md).

## Block-device scenarios

The additional modes are `block-persist`, `block-readonly`, `block-error`, `block-timeout` and `block-missing`. They require successful guest checks (QEMU exit 33) plus the host oracle. Persistence starts two separate VMs with one freshly created disposable disk. Phase logs, selected disk bytes and hashes are retained; the sparse disk itself is removed. See [BLOCK.md](BLOCK.md) for the contract and limits.
