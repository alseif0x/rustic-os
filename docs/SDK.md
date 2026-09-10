<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native Rust SDK and application manifest

Implemented for #11 on the R0 single-CPU x86_64 target. This is an allocation-free native application SDK over process ABI 1.0, IPC extension 1 and block extension 1, not a POSIX environment. [Process ABI](PROCESS-ABI.md), [IPC](IPC.md) and [loader limits](PROCESSES.md) remain authoritative.

## Boundaries and available API

| Component | Responsibility |
| --- | --- |
| `crates/abi` | Independent `no_std`, unsafe-free constants and manifest parser |
| `crates/sdk/src/arch.rs` | The single guest instruction boundary, synchronous INT 0x80 |
| `crates/sdk/src/startup.rs` | Entry macro, version check and exit on return |
| `crates/sdk/src/process.rs` | Version queries, identity, bounded integer diagnostics and exit |
| `crates/sdk/src/ipc/` | Owned fixed-size message encoding and endpoint send/receive/wait/close |
| `crates/sdk/src/block/` | Scoped disk geometry, copied request submission, result collection, wait/cancel/close |
| `apps/sdk-probe/` | Separate native IPC executable; imports only the public SDK |
| `apps/block-probe/` | Separate native storage executable; adversarial raw calls isolated in its test module |
| `tools/application.py` | Host build and strict TOML-to-binary manifest encoding |
| `kernel/src/process/runtime/application.rs` | Trusted admission before process allocation; used by the acceptance launcher |

The kernel depends on shared ABI contracts, never on the SDK. The test image embeds the independently linked ELF as opaque bytes; it does not link application Rust code as kernel functions. Runtime SDK calls are exposed only for `x86_64-unknown-none`; host tests exercise portable encoding and parsing without executing host INT 0x80.

`entry!(run)` accepts a function `fn(u64, u64, u64) -> u64`. The trusted launcher supplies RDI/RSI/RDX integers and a 64 KiB private stack with a guard page. The startup code clears the direction flag and calls Rust with the correct stack alignment; it checks both ABI versions before calling the application. Returning invokes EXIT. The example panic handler exits with 127; startup version mismatch exits with 126.

`process::report` is the existing eight-value integer diagnostic facility, not text output. `Endpoint::from_bootstrap` wraps a token without granting anything; the kernel validates owner, lifetime and rights on every operation. There is no public create/transfer-channel or kill-by-PID call.

Messages own 88 bytes of storage and expose at most 64 payload bytes. Outbound sender is zero; receive validates size, version, opcode and the kernel-supplied nonzero sender. Replies use a new message with the original correlation. Re-sending a received message directly is rejected because its sender field is populated. Receive is nonblocking; wait blocks for readiness and may return cancellation or closure. Close is explicit and consumes the wrapper; process exit remains the cleanup backstop. Handles are local tokens, not transferable authority when copied as integers.

## Memory and runtime limits

Rust `core`, stack variables, static data, slices and fixed arrays are available. Image segments retain the loader's R/RX/RW permissions. The SDK has no allocator, allocation/free syscalls, `alloc`, `std`, networking, threads, TLS, floating point or SIMD support. It does not publish placeholders for those functions. The pinned target avoids a red zone and SIMD. Do not add dependencies that require a runtime the OS has not implemented.

A later service adds a cohesive client module when its actual contract exists, conforming to the [versioned service schemas](SERVICE-CONTRACTS.md) from #6. The initial schemas and host descriptors do not add native service calls. #44 supplies the lower-level [block API](BLOCK-ACCESS.md); The [bounded file-service protocol and clients](FILES.md) now support the terminal; [native tracked replacements and result lookup](FILE-RECOVERY.md) now recover retained results after restart; complete service-v1 conformance remains in #12/#22. A 512-byte block is separate from the logical file-operation payload limit. Service versions remain separate from the process/IPC ABI. MCP is an adapter above services and does not block native SDK use.

## Manifest schema 1

The editable descriptor is [app.toml](../apps/sdk-probe/app.toml). The host encoder rejects unknown fields, unsupported versions, paths, duplicate/unknown requests and malformed versions before compiling. Text input is limited to 4096 bytes.

The kernel independently parses exactly 128 bytes with explicit little-endian decoding. No Rust struct layout, references, padding or pointers cross this boundary.

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 8 | Magic `RUSTAPP\0` |
| 8 | 2 | Manifest schema, 1 |
| 10 | 2 | Total length, 128 |
| 12 | 4 | Required process ABI, 65536 (1.0), exact match |
| 16 | 2 | Required IPC extension, 1, exact match |
| 18 | 6 | Application major/minor/patch, three u16 integers |
| 24 | 8 | Capability requests: bit 0 IPC, bit 1 diagnostic, bit 2 block, bit 3 console, bit 4 supervisor control; unknown bits rejected |
| 32 | 32 | Application identity |
| 64 | 32 | Executable filename, ending in `.elf` |
| 96 | 32 | Reserved, all zero |

Names contain 1–31 ASCII bytes, start with a lowercase letter, and otherwise allow lowercase letters, digits, dot, underscore and hyphen. A terminating zero and all-zero remainder are required. Executable names are single filenames; slash and directory traversal paths cannot enter the descriptor.

Admission rejects a mismatch between the manifest's executable name and the trusted launcher's selected name, unavailable requests, invalid manifest/ABI versions and invalid ELF before execution. Name and version are descriptive metadata, not authenticated publisher identity. This manifest provides no signature, package integrity or installation system.

**Requests never grant authority.** The caller supplies the available feature set for admission and must separately provision actual handles. The test launcher creates an endpoint for each process and supplies its token. Diagnostics are already a bounded ambient syscall; the diagnostic request states a requirement, not a new per-app enforcement mechanism. [ADR-0002](architecture/ADR-0002-authority-and-delegation.md) defines the authority baseline; the [native runtime](NATIVE-RUNTIME.md) now supplies the trusted supervisor identity; file services enforce scoped grants, expiry and revocation. Broader #13 authority cases remain open.

## Build and run the example

From the repository root in the pinned Ubuntu 24.04 environment:

```sh
source ~/.cargo/env
python3 tools/application.py
# Outputs: target/native/sdk-probe.elf, app.manifest, block-probe.elf, block-probe.manifest

cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode ok
```

The image builder compiles the application first, then builds the kernel with `sdk-test` and an explicit `RUSTIC_APPLICATION_DIRECTORY`. No nested Cargo invocation runs in the kernel build script. `cargo xtask check` also builds/lints the guest application before checking the test image. A manual `boot-image` build alone omits SDK acceptance and cannot pass the full `ok` suite.

The app linker script emits a static ET_EXEC with separate PT_LOAD segments at 0x400000. No ELF parser relaxation was required. The observed release executable is 20,184 bytes; this is a measurement with Rust 1.98.1, not a fixed format requirement. The loader continues to enforce its 1 MiB file, 256-page process and W^X limits.

To create another native utility, follow the example's Cargo package, linker script, entry function and panic handler. Add it to the workspace and explicitly extend the host build/launcher to select it. The builder selects six applications: two probes, supervisor, file server, shell and utility. The shell launches fixed utility roles through the supervisor. Arbitrary discovery, installation and executable-file launch remain later work.

The example runs two instances with opposite roles. They verify identity, reject an oversized payload and invalid handle, exchange four correlated requests/replies with authenticated sender IDs, close their endpoints and report completion. Malformed example roles or peer identities return a nonzero exit code.

## Acceptance and provenance

The guest acceptance asserts twelve admission failures: eight corrupted fields, truncated manifest, wrong executable selection, unavailable requests and invalid ELF. None leaks frames. Two independent SDK applications then execute in ring 3, verify four exchanges and four parameter rejections in total, exit successfully and release every process frame, channel and handle. Serial output includes `RUSTIC SDK verified=1`, executable size, peak simultaneously allocated process frames and before/after free frames.

Both direct and isolated suites require the complete SDK summary, in addition to existing process, IPC, memory and interrupt acceptance. Host regression tests reject missing, duplicated or incomplete evidence. At #11 acceptance there were 33 Rust and 22 Python tests, with 13 VM and 17 executor scenarios. SDK checks remain inside `ok`; [block storage](BLOCK.md) expands the current suites.

Direct image metadata records the application ELF and manifest hashes as well as the containing kernel/image hashes. The isolated executor builds from an exact revision, exports bounded application/manifest artifacts and records their hashes. The separate trusted boot worker executes the resulting kernel; a successful compiler output alone is insufficient. Published revision and CI evidence are recorded in [#11](https://github.com/alseif0x/rustic-os/issues/11).

External dependencies remain the pinned host Rust/compiler/linker, Python, Limine, QEMU and OVMF described in [the inventory](dependencies.md). No new third-party crate, code template or runtime was introduced. The SDK and applications are original Apache-2.0 code. Compiler, linker and VM run on the host; the application instructions run inside RusticOS. Review was performed by the implementing agent, without an independent audit.

## Correlated RPC and independent control

The native `rpc::Rpc` exposes `begin` (one nonblocking send), `poll` (at most one reply), `pending` and `failed`. One request may be active per binding; WouldBlock during admission consumes no correlation and means no request was sent. A successful admission retains its correlation until the authenticated peer replies. No automatic resubmission occurs. The allocation-free `rpc::state` module owns admission/correlation/poisoning facts and can be tested on the host; the guest client owns transport and waits.

`exchange`/`words` remain bounded synchronous conveniences (1000 PIT ticks, nominally ten seconds on R0). An abandoned response, transport failure or RPC peer/correlation mismatch poisons the binding permanently; callers must obtain a fresh authorized binding. A later typed read validation failure (opcode, context, metadata or hash) returns `Protocol` and clears the output; it does not itself poison the RPC stream. A late response cannot complete a subsequent mutation. The supervisor uses begin/poll for [revocation and actor control](TAKEOVER.md), retaining unconfirmed work to accept a valid late acknowledgment without blocking owner queries. An observation deadline does not cancel admitted work. Caller-owned `Progress` supplies the waiting policy; the default remains `Blocking`, while the shell can interrupt foreground waits without coupling SDK transport to console input. Native file `submit`/`poll` retain one packet and its opcode/context. Supervisor provisioning, policy and mount now use incremental jobs; see [foreground control](FOREGROUND-CONTROL.md). [Stable references and version-pinned reads](FILES-READ.md) use the same RPC and caller-owned progress. Full service-v1 asynchronous operation and cancellation semantics remain separate work.

## Bounded block client

#44 adds `block::Device`. The trusted launcher supplies its handle; `from_bootstrap` wraps it without granting rights. Query the independent block wire version with `Device::version()`. `read`, `write` and `flush` return a request ID after admission; `wait(id)` yields until completion, and `result()` copies and consumes the retained result. Check completion status/effect as well as syscall success. Write admission snapshots exactly 512 bytes, so the source slice need not live until completion. Read buffers are supplied only during collection.

`cancel(id)` returns true only for cancellation before device submission; false means too late and requires reading the actual result. Closing a handle consumes the wrapper but does not roll back submitted writes. Errors, byte layouts, quotas, scope, resource recovery, both new VM modes and their measured evidence are specified in [BLOCK-ACCESS.md](BLOCK-ACCESS.md). The example is independently linked and executes under RusticOS; the host encoder/compiler does not perform its I/O.
