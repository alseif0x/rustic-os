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

`rustic-abi` 0.1.0 is an original crate in this repository under Apache-2.0, without external dependencies or unsafe code. It contains process/IPC/block constants, errors, wire codecs and the application manifest contract; the kernel and SDK consume it independently. Cargo.lock adds only that path dependency: limine and bitflags versions/checksums remain unchanged. Existing image notices remain sufficient; no new third-party material is included.

The original `rustic-sdk` and `rustic-sdk-probe` packages added for #11 also use Apache-2.0. They add only path dependencies on the shared ABI/SDK, without external crates. The test kernel embeds the separately linked application ELF; the existing Rust runtime notice also covers that application.

## Block storage implementation

#35 adds original Apache-2.0 driver/DMA code, following the [VirtIO specification](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html) legacy PCI contract. The [virtio-drivers README](https://github.com/rcore-os/virtio-drivers) was reviewed as a reuse option; the crate was not incorporated and no source was copied. The [transport decision](BLOCK.md) records the tradeoff. Existing QEMU/OVMF packages and Cargo dependencies remain unchanged. Sparse test disks contain only generated fixtures.

## Architecture research tooling

The finite operation model in `tools/research/operation_model` is original Apache-2.0 Python code using only the reference host's standard library. The [research agenda](architecture/systems-roadmap.md) cites prior work for design comparison; no source from those projects or papers is copied or distributed. No new package, Rust crate or guest runtime is introduced.

## Host service contract checks — 2026-09-10

The original `tools/contracts` code and schemas use Apache-2.0. The validator uses these external Python packages from PyPI in a local/CI virtual environment. [requirements.txt](../tools/contracts/requirements.txt) pins every version and downloaded wheel SHA-256 for Ubuntu 24.04 amd64 / CPython 3.12. No wheel, environment, package source or compiled extension is included in the repository, boot image, executor image or exported evidence. Generated descriptors contain original project schemas.

| Package / source | Version | License and installed notice | Role |
| --- | --- | --- | --- |
| [jsonschema](https://github.com/python-jsonschema/jsonschema) | 4.26.0 | MIT; dist-info/licenses/COPYING | Draft 2020-12 validation |
| [attrs](https://github.com/python-attrs/attrs) | 26.1.0 | MIT; dist-info/licenses/LICENSE | Validator data structures |
| [jsonschema-specifications](https://github.com/python-jsonschema/jsonschema-specifications) | 2025.9.1 | MIT; dist-info/licenses/COPYING | Offline meta-schema registry |
| [referencing](https://github.com/python-jsonschema/referencing) | 0.37.0 | MIT; dist-info/licenses/COPYING | Local reference resolution |
| [rpds-py](https://github.com/crate-py/rpds) | 2026.6.3 | MIT; dist-info/licenses/LICENSE | Host persistent structures, external native wheel |
| [typing_extensions](https://github.com/python/typing_extensions) | 4.16.0 | PSF-2.0; dist-info/licenses/LICENSE | Host type support |

Implementer reviewed the installed metadata and supplied license files against the selected distributions; no independent audit. Their original notices remain in the environment. External development use introduces no new linked guest license obligation; bundling any of these packages later requires preserving the complete package notices and reviewing compiled transitive components. Other architectures/Python versions need their own reviewed wheel hashes rather than bypassing the hash check. Cargo.lock and existing boot-image notices are unchanged.

#44 adds the original Apache-2.0 `rustic-block-probe` path package and typed block codecs/SDK clients. It depends only on `rustic-sdk`; Cargo.lock retains all existing external versions and checksums. The separately linked probe uses the same Rust runtime notice as the SDK probe. No third-party code or additional package is introduced.

## Native terminal increment — 2026-09-10

The original rustic-fs, rustic-file-service, rustic-file-server, rustic-supervisor, rustic-shell and rustic-utility packages use Apache-2.0 and path dependencies only. No additional external Cargo package was added; limine/bitflags and existing image notices are unchanged. The bounded volume implementation is original Rust code informed by the sources in FILES.md; no littlefs, FAT or SQLite source was copied and format compatibility is not claimed. Host terminal/acceptance code uses only the reference Python standard library.
