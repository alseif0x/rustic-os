<!-- SPDX-License-Identifier: Apache-2.0 -->

# Browser feasibility: early runtime investigation

Date: 2026-09-11. Status: initial static preflight for [#7](https://github.com/alseif0x/rustic-os/issues/7), not an engine selection or a completed browser experiment. The local engine/rendering and W1–W5 requirements in [v0.1](../requirements-v0.1.md) remain mandatory. H4 is the acceptance milestone; investigation starts now because runtime requirements can affect H1–H3 decisions.

## Question and alternatives

What is the smallest maintainable path to a browser that meets the agreed web fixtures on RusticOS? Compare an embedded engine with a deliberate native platform layer against a compatibility layer for a specific application. Neither “written in Rust” nor ELF support establishes compatibility with our `no_std` SDK, ABI, memory limits or services.

| Candidate | Why investigate | Known boundary / present status |
| --- | --- | --- |
| Servo | Rust engine with an embedding API and a separate demonstration shell | Official platforms require substantial runtime support. First candidate pinned and statically inspected below; embedding and feature reduction still need experiments. |
| NetSurf | Portable frontends, including framebuffer support, make it a useful smaller comparison | Has JavaScript support, but that does not establish the agreed HTML/CSS/JS fixtures. Pin a revision and measure those fixtures before proposing it as the product browser. |
| Ladybird | Independent browser with its own platform abstractions; useful implementation reference | Current desktop work targets Linux/macOS, with a substantial C++ codebase and incremental Rust adoption. No artifact or native port evaluated here. |

Primary sources: [Servo book](https://book.servo.org/), [Servo Linux dependencies](https://book.servo.org/building/linux.html), [NetSurf architecture and supported platforms](https://www.netsurf-browser.org/about/), [Ladybird project](https://ladybird.org/). Candidate order is an investigation decision, not a quality ranking. No dependency is added to RusticOS by this review.

## Completed probe: pinned Servo release

Budget for this first probe: one official release archive, one static ELF inspection, no source build or engine execution. This tests the narrow claim that the available Linux binary might already run through our loader.

- Candidate: `Servo v0.5.0`, source commit `1d44e5dd6a8b64c02f9dbf7fcbdf4ebdd0740019`.
- [Official release](https://github.com/servo/servo/releases/tag/v0.5.0), published 2026-08-31; archive `servo-x86_64-linux-gnu.tar.gz`, 49,966,475 bytes.
- Archive SHA-256: `6a3d536b07bed16d4833012162cde5285740614d5ff2855dfc36e60576779170`, checked against the published asset digest.
- Selected member: `servo/servoshell`, 134,254,352 bytes; SHA-256 `faf26238342f3db1da7550135ac18aa8c1080007d58c0ddf6ac85563573f9710`.
- Inspector: GNU `readelf` 2.42 on Ubuntu 24.04/WSL2. The executable was inspected, **not run**. No guest or supported-host web fixture passed as part of this probe.

The [structured observation](../evidence/servo-0.5.0-preflight.json) records the ELF requirements. It is an ELF64 x86_64 `ET_DYN` PIE with four load segments, twelve program headers, a thread-local-storage segment and interpreter `/lib64/ld-linux-x86-64.so.2`. Dynamic dependencies include libc, libstdc++, libudev, fontconfig, GLib and several GStreamer libraries. ELF `PT_TLS` means thread-local storage, not HTTPS/TLS.

The current [native loader](../../kernel/src/process/elf.rs), inspected at `79ae9091bb19dbe2a3b3c8d830ad00f36bdde548`, accepts bounded static `ET_EXEC` input of at most 1 MiB. RusticOS does not provide this interpreter or Linux process/library ABI. The stock binary therefore cannot load directly. Increasing the byte limit alone cannot fix these mismatches. File size is neither resident RAM nor the minimum size of a custom embedded build.

This rejects direct execution of this artifact, **not Servo as an engine**. The demonstration shell's default media/windowing dependencies need not all be mandatory for a different embedding. Inspect the pinned [shell manifest](https://github.com/servo/servo/blob/1d44e5dd6a8b64c02f9dbf7fcbdf4ebdd0740019/ports/servoshell/Cargo.toml) and [engine manifest](https://github.com/servo/servo/blob/1d44e5dd6a8b64c02f9dbf7fcbdf4ebdd0740019/components/servo/Cargo.toml) before inferring the smallest necessary dependency set.

### Reproduction

Run from the repository root on the documented Ubuntu host. These commands only download and inspect a public release; they do not install or execute it.

```sh
mkdir -p artifacts/browser-feasibility
curl --fail --location --max-time 90 \
  https://github.com/servo/servo/releases/download/v0.5.0/servo-x86_64-linux-gnu.tar.gz \
  --output artifacts/browser-feasibility/servo-v0.5.0.tar.gz
echo '6a3d536b07bed16d4833012162cde5285740614d5ff2855dfc36e60576779170  artifacts/browser-feasibility/servo-v0.5.0.tar.gz' | sha256sum --check
tar -xOzf artifacts/browser-feasibility/servo-v0.5.0.tar.gz servo/servoshell \
  > artifacts/browser-feasibility/servoshell
echo 'faf26238342f3db1da7550135ac18aa8c1080007d58c0ddf6ac85563573f9710  artifacts/browser-feasibility/servoshell' | sha256sum --check
readelf --version
readelf -h -l -d artifacts/browser-feasibility/servoshell \
  > artifacts/browser-feasibility/readelf.txt
```

The source [license](https://github.com/servo/servo/blob/1d44e5dd6a8b64c02f9dbf7fcbdf4ebdd0740019/LICENSE) is MPL-2.0. That identifies the engine source license, not every bundled dependency's terms. Before importing or distributing any component, record its exact configuration and notices under [LICENSING.md](../LICENSING.md). The archive and executable remain local ignored artifacts; only our observations and reproduction instructions are published.

## Next bounded experiment

Work budget: two engineering hours for the next probe, with a 20-minute ceiling on any single build and explicit process/resource limits. This is a stopping rule, not a delivery estimate. Record setup failure or budget exhaustion and choose the next candidate/probe; do not spend days silently building a browser.

1. Run the pinned release on a supported host, against a versioned local fixture set derived from W1–W4. Record rendering, JavaScript results, requests, exit status and process/RAM observations. Host success establishes only the candidate baseline; W5 and all guest evidence remain open.
2. From the pinned source, inventory required Rust `std`, C/C++, threading/synchronization, executable memory, text/fonts, surface/input, files and networking/TLS interfaces. Separate engine requirements from optional shell/media features. Do not implement a generic Linux ABI from the `DT_NEEDED` list.
3. Select one concrete portability seam before coding: for example a minimal offscreen surface/text embedding and its allocator/thread/runtime calls. Compile or run the smallest probe that actually exercises that seam. If baseline setup consumes the budget, report that result and separately budget the seam experiment; do not call the combined acceptance complete.
4. Publish measured gaps with an owner issue and a pass/fail test, reviewing their impact on #3/#11/#16/#17/#18/#19. A broad label such as “add POSIX” is not an implementation task. Reuse decisions belong to the consuming application and service boundary.

Closure for #7 still requires a supported-host experiment **and** evidence for the most uncertain portability boundary, an explicit viable path/alternative/blocker, dependency/license inventory, and specific integration blockers for #19. If Servo fails the web or adaptation budget, pin NetSurf for the same relevant fixtures before expanding the candidate list. A remote browser, screenshot service or new engine written from scratch does not satisfy the product requirement.
