<!-- SPDX-License-Identifier: Apache-2.0 -->

# Licensing and provenance

## Project license

Original RusticOS code and documentation use **Apache License 2.0**, as decided by the owner in [#32](https://github.com/alseif0x/rustic-os/issues/32). The complete official text is preserved in [LICENSE](../LICENSE), without replacing its example fields or adding custom terms.

The original license publication was verified against the [official Apache text](https://www.apache.org/licenses/LICENSE-2.0.txt), with SHA-256 `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`. Issue #32 retains the publication history and evidence.

## Current distribution

The repository contains original kernel code, the shared ABI crate, host tools and documentation, plus preserved third-party license notices. The boot image includes Limine's EFI loader and code from the Rust bindings/runtime.

The [dependency inventory](dependencies.md) records component versions, provenance, use and distribution limits. Full dependency notices are retained under `licenses/` and copied into the image's `/licenses` directory alongside the project's license. Third-party texts and components retain their own terms; they are not relicensed under Apache-2.0.

QEMU and OVMF remain external tools. OVMF is not included in the boot image or CI artifacts. The locally built executor image is not published to a registry.

## Notices

The current distribution preserves required notices through the component license files; there is no project-level NOTICE file. Reevaluate this when adding material with required attributions. Add NOTICE or another appropriate attribution where required; Apache-2.0 does not authorize removing third-party notices.

## Before adding or changing a dependency

Record for each component:

- Name, exact version/revision and source.
- License identifier and required notices.
- Build/runtime use and source/binary distribution.
- Compatibility assessment, responsible reviewer and evidence.

Resolve blockers before distribution. Do not assume an open license is sufficient for every combination. Preserve original headers and links to license texts/notices, and clearly identify exceptions to the project license.

Repeat review when a dependency or its distribution changes. Approval of the current inventory does not approve future dependencies.

## Source-file convention

Original files with comment syntax use `SPDX-License-Identifier: Apache-2.0`, as described in [CONTRIBUTING.md](../CONTRIBUTING.md). Use syntax appropriate to the file format; do not add comments to formats that cannot contain them.

Copyright notices must reflect actual authorship. Preserve third-party headers and keep the official LICENSE text unchanged.
