<!-- SPDX-License-Identifier: Apache-2.0 -->

# Dependency and tool inventory

Date: 2026-09-10. Reviewed by the implementing agent; no independent audit. Complements [LICENSING.md](LICENSING.md).

Cargo.lock contains the original workspace packages, limine 0.5.0, bitflags 2.13.1 and the reviewed SHA-256 dependency graph below. Original code is Apache-2.0; dependency crates retain their licenses. The boot image includes Limine's BOOTX64.EFI and the project's ELF with separately linked application images; OVMF firmware remains external.

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

## Native file range hashing — 2026-09-10

The native `files.read` binding uses RustCrypto's SHA-256 implementation in `rustic-file-service` and `rustic-sdk`. The workspace pins `sha2 = "=0.11.0"` with default features disabled; Cargo.lock fixes all transitive versions and archive checksums. Hashing belongs to the service and client implementation; `rustic-abi` and the kernel retain no hashing dependency. A range hash covers returned bytes and detects corruption or response mismatch; it does not authenticate a publisher or confer file authority.

The implementing dependency-review agent checked the official [sha2 manifest](https://docs.rs/crate/sha2/0.11.0/source/Cargo.toml), [backend selection](https://docs.rs/sha2/0.11.0/sha2/), resolved Cargo metadata and the complete notices in registry archives verified against Cargo.lock. This is a dependency/provenance review, not a cryptographic audit. Reusing the maintained implementation avoids introducing an original cryptographic primitive; the existing filesystem CRC is not SHA-256. No third-party source was copied into first-party Rust modules.

| Component / official crate source | Version / archive SHA-256 | Preserved notice |
| --- | --- | --- |
| [sha2](https://crates.io/crates/sha2/0.11.0) | 0.11.0 / `446ba717509524cb3f22f17ecc096f10f4822d76ab5c0b9822c5f9c284e825f4` | [sha2-MIT.txt](../licenses/sha2-MIT.txt) |
| [digest](https://crates.io/crates/digest/0.11.3) | 0.11.3 / `f1dd6dbb5841937940781866fa1281a1ff7bd3bf827091440879f9994983d5c2` | [digest-MIT.txt](../licenses/digest-MIT.txt) |
| [block-buffer](https://crates.io/crates/block-buffer/0.12.1) | 0.12.1 / `d2f6c7dbe95a6ed67ad9f18e57daf93a2f034c524b99fd2b76d18fdfeb6660aa` | [block-buffer-MIT.txt](../licenses/block-buffer-MIT.txt) |
| [crypto-common](https://crates.io/crates/crypto-common/0.2.2) | 0.2.2 / `ce6e4c961d6cd6c9a86db418387425e8bdeaf05b3c8bc1411e6dca4c252f1453` | [crypto-common-MIT.txt](../licenses/crypto-common-MIT.txt) |
| [hybrid-array](https://crates.io/crates/hybrid-array/0.4.15) | 0.4.15 / `27f864f10dfb56725ce5ce5472bc52252c8f93a4ab86327122cebf62c5f59a17` | [hybrid-array-MIT.txt](../licenses/hybrid-array-MIT.txt) |
| [typenum](https://crates.io/crates/typenum/1.20.1) | 1.20.1 / `b6f5e870be6c3b371b77fe0ee0bafb859fa4964b4404c27de1d380043c4dda20` | [typenum-MIT.txt](../licenses/typenum-MIT.txt) |
| [cfg-if](https://crates.io/crates/cfg-if/1.0.4) | 1.0.4 / `9330f8b2ff13f34540b44e946ef35111825727b38d33286ef986142615121801` | [cfg-if-MIT.txt](../licenses/cfg-if-MIT.txt) |
| [cpufeatures](https://crates.io/crates/cpufeatures/0.3.1) | 0.3.1 / `5ca28b0ae3115b884660db4118d803791fd6756b6e88f39c0f3f7859060d7566` | [cpufeatures-MIT.txt](../licenses/cpufeatures-MIT.txt) |

All eight crates declare `MIT OR Apache-2.0`; this distribution preserves and uses their MIT option. The listed files are byte-for-byte copies of each package's `LICENSE-MIT`, including its original copyright holders. No additional NOTICE/COPYING file was present in those reviewed archives. They are included in the boot image's `/licenses` directory with the existing notices. Source and binary distribution under these selected terms is compatible with the project's Apache-2.0 distribution while retaining the component terms.

The selected x86_64 guest graph enables `digest/block-api` and `typenum/const-generics` plus dependency default feature groups that add no allocation or OS dependency. It enables no `sha2` default, `alloc`, `oid` or `zeroize` feature, and no `std` or randomness feature. The maximum declared MSRV is 1.85, below the pinned Rust 1.98.1 toolchain. `.cargo/config.toml` forces `sha2_backend="soft"` only for `x86_64-unknown-none`, preserving its static relocation setting. R0 traps FP/SIMD instructions, so hardware detection alone cannot authorize SHA-NI/SSE/AVX use. The current selection uses the standard software implementation; no compact-code optimization or new first-party unsafe boundary is introduced. Transitive libraries retain their internal implementation boundaries and are not claimed to be entirely free of unsafe code.

Cargo.lock also records [libc 0.2.189](https://crates.io/crates/libc/0.2.189), archive SHA-256 `3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2`, MIT OR Apache-2.0. It is a `cpufeatures` dependency only on selected non-x86 platforms and is absent from both the current `x86_64-unknown-none` graph and x86_64 host graph. It is fetched into the local, unpublished executor registry cache with its original notices; no libc code or crate archive is included in the boot image. Expanding platform or feature support requires a fresh dependency and notice review.

The existing build identity includes Cargo.lock and `.cargo/config.toml`. Executor preparation fingerprints the entire copied reference, including manifests, lockfile, configuration, notices and image packager, and fetches the locked registry graph before offline candidate execution. Re-run `python3 tools/sandbox.py prepare` after this change so the trusted image contains the new packages and notices. Candidate jobs remain offline; a missing dependency is a build failure, never permission to download it. No registry image is published and no source/build cache hash bypass is introduced.
