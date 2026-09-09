<!-- SPDX-License-Identifier: Apache-2.0 -->

# Licensing and provenance

## Project decision

The owner selected Apache License 2.0 for original RusticOS material in [#32](https://github.com/alseif0x/rustic-os/issues/32). The full official text is preserved in [LICENSE](../LICENSE), without replacing its example fields or adding custom terms.

Source: [official Apache text](https://www.apache.org/licenses/LICENSE-2.0.txt). SHA-256 of the official file downloaded for this publication: `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`.

## Initial baseline — September 9, 2026

The initial license publication contains LICENSE and original documentation. It does not include kernel code, crates, runtime/build dependencies, binaries, firmware, fonts or vendored material.

| Category | Status at that initial publication |
| --- | --- |
| Original code and Rust dependencies | Not yet included. |
| Original documentation | Apache-2.0, identified through SPDX. |
| Distributed third-party components/material | None; LICENSE reproduces the official license text. |
| External tools used to edit/publish | Not distributed as part of RusticOS. Their use does not establish review of future OS dependencies. |

## NOTICE

The workspace delivery adds original code and external development tools. The [dependency inventory](dependencies.md) records their versions, provenance and distribution limits; the initial baseline above is retained as the history of the license publication.

No initial NOTICE file was added because that publication included no third-party component attributions requiring one. This decision applies only to that publication's contents. When material with required notices is introduced, preserve them and add NOTICE or another attribution as appropriate; Apache-2.0 does not authorize removing them.

## Review before adding a dependency

Record for each component: name, version/revision, source, license identifier, build/runtime use, source/binary distribution, required notices, compatibility assessment, responsible reviewer and evidence.

Resolve blockers before distribution; do not assume an open license is sufficient for every combination. Third-party material retains its own terms. Preserve original headers and links to license texts/notices; clearly separate any exception to the project's license.

Repeat the review when a dependency or its distribution changes. The initial empty baseline does not approve future dependencies.

## Convention

The initial #8 image includes Limine in the volume and bindings/runtime in the ELF. The [current inventory](dependencies.md) identifies versions and licenses; the image preserves their full notices under /licenses. OVMF and QEMU remain external tools. The initial absence of third parties described above is historical, not a description of this image.

Original files with comment syntax use `SPDX-License-Identifier: Apache-2.0`, as described in [CONTRIBUTING.md](../CONTRIBUTING.md). Copyright notices must reflect actual authorship. The LICENSE text remains unchanged.
