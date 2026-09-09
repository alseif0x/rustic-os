<!-- SPDX-License-Identifier: Apache-2.0 -->

# Dependency and tool inventory

Date: 2026-09-09. Reviewed by the implementing agent; no independent review. Complements [LICENSING.md](LICENSING.md).

Cargo.lock contains the original rustic-kernel, rustic-abi, rustic-sdk, rustic-sdk-probe and xtask packages, plus limine 0.5.0 and bitflags 2.13.1. Original code is Apache-2.0; dependency crates retain their licenses. The boot image includes Limine's BOOTX64.EFI and the project's ELF; OVMF firmware remains external.

| Component | Version / source | Use and distribution |
| --- | --- | --- |
| Rust/Cargo/rust-lld | Rust 1.98.1, commit 48a229ceaefd4985c50990b14116b6d856af0985; LLVM 22.1.8; static.rust-lang.org | External toolchain; not redistributed in the repo. Rust uses MIT/Apache-2.0; LLVM retains its terms and exceptions. |
| rustup | 1.29.1, official installation | External manager, not distributed. |
| QEMU | 1:8.2.2+ds-0ubuntu1.18, Ubuntu noble | External executor, GPL-2.0 and components with their own notices; not distributed or linked into the kernel. |
| OVMF | 2024.02-2ubuntu0.9, Ubuntu noble | External firmware; component notices in Ubuntu/EDK II packages. Not distributed. |
| mtools / dosfstools / xorriso | Exact versions in tools/environment.toml, Ubuntu noble | External utilities; preserve their terms if distributed in future. |
| Limine | 12.8.0, [official release](https://github.com/Limine-Bootloader/Limine/releases/tag/v12.8.0) | BOOTX64.EFI extracted from the verified archive and included in the image. The complete BSD-2-Clause LICENSE from Mintsuki and contributors is copied to /licenses/LIMINE.txt. |
| checkout / upload-artifact | SHA in workflow; official actions repositories | Remote CI actions, not vendored. They do not license the project's code. |
| Docker Engine / Ubuntu container | Engine 29.7.2 validated locally; Ubuntu 24.04 amd64 with digest in tools/sandbox_support/prepare.py | External #21 tools. Image built locally, without registry publication; each package retains its notices. Final identity is stored per job. |

tools/environment.toml preserves the Limine archive URL/hash, OVMF hashes and protocol revision. OVMF is not packaged in the image or CI artifacts. Original dependency notices are preserved in licenses/ and inside the image; those texts are not relicensed under Apache-2.0.

## Third-party code included in the kernel ELF

| Component | Version/lockfile checksum | Distributed notice |
| --- | --- | --- |
| limine, Rust bindings | 0.5.0 / af6d2ee42712e7bd2c787365cd1dab06ef59a61becbf87bec7b32b970bd2594b | MIT OR Apache-2.0; LICENSE-MIT preserved in licenses/limine-rust-MIT.txt. |
| bitflags | 2.13.1 / b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da | Transitive no_std dependency; MIT preserved in licenses/bitflags-MIT.txt. |
| Rust core/runtime | Toolchain 1.98.1 | Official tag's LICENSE-MIT preserved in licenses/rust-MIT.txt. |

These licenses are copied to /licenses alongside RUSTIC.txt (the project's Apache-2.0 license). The pure library still has no Limine dependency; bindings are enabled only for the boot-image binary. No kernel-template code is copied. Limine 0.6.5 was inspected and rejected because it needs experimental ptr_metadata; it is not in the artifact. limine 0.5.0 is tested with base revision 3 and loader 12.8.0.

External component review sources: LICENSE from the hash-verified Limine archive; installed package metadata and copyright under /usr/share/doc; [Rust copyright](https://github.com/rust-lang/rust/blob/master/COPYRIGHT), [QEMU license](https://www.qemu.org/docs/master/about/license.html). The specific terms of a future distributed package are reviewed before publication.

## Original shared contracts

`rustic-abi` 0.1.0 is an original crate in this repository under Apache-2.0, without external dependencies or unsafe code. It contains process/IPC constants, errors and the application manifest contract; the kernel and SDK consume it independently. Cargo.lock adds only that path dependency: limine and bitflags versions/checksums remain unchanged. Existing image notices remain sufficient; no new third-party material is included.

The original `rustic-sdk` and `rustic-sdk-probe` packages added for #11 also use Apache-2.0. They add only path dependencies on the shared ABI/SDK, without external crates. The test kernel embeds the separately linked application ELF; the existing Rust runtime notice also covers that application.
