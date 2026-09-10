<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native stable references and bounded reads

This increment connects the logical [`files.read` contract](SERVICE-CONTRACTS.md) to the real file service, typed Rust SDK, manual shell and deterministic native clients. It reads at most 1,024 bytes, identifies the observed version and hashes exactly the returned range. No model, network, image interpretation or MCP server is involved.

This guide covers the native read operation. The separate [completed-operation binding](FILE-OPERATIONS.md) now implements bounded files.replace and operations.get through the same service and SDK. The eight-operation catalog remains specified_not_implemented as a complete surface: live discovery, general asynchronous lifecycle, cancellation and events retain their own implementation work in #12/#13/#22/#43.

## Identity and authority

A workspace reference combines the persistent volume lineage with the ID of a selected directory. A resource reference includes that workspace and the object's ID. The directory may be a fixed namespace root or a nested directory; the virtual `/` object is not a workspace. Selecting another ancestor for the same object produces a different workspace-qualified reference.

Object IDs increase durably and are not reused when a metadata slot becomes free. Removing and recreating a file or workspace under the same name produces a new identity. Service restart and ordinary VM reboot preserve the surviving references and versions. A recreated volume has a new lineage. Offline rollback/clone resistance remains outside the storage guarantee; a restored store that cannot preserve its lineage needs a new identity.

These values identify objects and never grant access. Every native request authenticates the endpoint's actual peer, context generation and current rights, including expiry/revocation. The file service checks both the granted object scope and membership in the selected workspace before exposing file facts. Naming an ancestor workspace does not broaden a file-only grant. Unknown, removed and foreign references receive the same conservative denial on this binding; it does not expose existence outside an authorized scope.

Fresh authority is necessary after service restart or regrant. Reusing a stable reference cannot reopen an old endpoint, restore a helper or resume an interrupted task automatically.

The native text encoding is canonical lowercase ASCII:

| Value | Encoding | Length |
| --- | --- | --- |
| Workspace | `ws_` + 32 hex lineage digits + `_` + 8 hex directory-ID digits | 44 |
| Resource | `rs_` + 32 hex lineage digits + `_` + 8 hex directory-ID digits + `_` + 8 hex object-ID digits | 53 |
| Version | `v_` + 16 hex digits | 18 |
| Retry epoch | `e_` + 16 hex digits | 18 |

Zero lineage, IDs, versions and epochs are invalid. Parsers reject uppercase alternatives, changed separators, whitespace, truncated forms and extra bytes. Workspace and resource fields must describe the same workspace. These encodings fit the logical schema's 64-character token bound; they are not integer kernel handles.

The returned retry epoch is an observation of the volume current retention generation. Read-only clients, including helpers without a recovery subject, may receive it after a permitted read. It does not grant receipt inspection, replacement or cancellation. The [workspace replacement binding](FILE-OPERATIONS.md) consumes this epoch with a canonical key; its retry namespace is workspace-scoped, but the epoch and two retained slots remain volume-wide. The e_ value is not a complete token for the legacy replace/receipt commands.

## Manual use

Start the [native terminal](TERMINAL.md), then create a small example:

```text
mkdir api-demo
write api-demo/note "Hello from RusticOS"
ref api-demo api-demo/note
```

`ref WORKSPACE_PATH FILE_PATH` resolves the existing paths and prints `workspace=... resource=...`. Copy those exact values into the following template; the shell does not expand placeholder names:

```text
read-ref WORKSPACE_REFERENCE RESOURCE_REFERENCE - 0 5
```

The arguments are workspace reference, resource reference, expected version, byte offset and requested length. `-` selects the version observed at the beginning of the read. A successful command prints version, size, offset, returned length, EOF, retry epoch and range SHA-256, followed by `data=` containing hexadecimal bytes. This presentation preserves binary bytes; the logical JSON representation uses canonical base64 instead.

For a subsequent range of the same file version, replace `-` with the returned `v_...` token. A later edit yields `Version` rather than silently combining ranges from different versions. `stat` and the legacy tracked-replacement command still use numeric versions; their command syntax is unchanged.

`cat PATH` now uses the same `Client::references` and `Client::read_range` SDK methods, selecting the file's namespace root as its workspace. It prints verified bytes using the existing safe terminal presentation. Only a legacy store without the required lineage/recovery metadata permits a fallback to the older native read. Other errors, including denial, corruption and version conflict, propagate. `ref` and `read-ref` remain `Unavailable` on that legacy store until the owner performs the [explicit upgrade](FILE-RECOVERY.md); reading a file never upgrades or reformats its volume.

The deterministic `act PID api-read` action also uses the full SDK path. `act PID read-open` and `read-next` expose a controlled pause between chunks for owner-edit/revocation tests. `act PID fill` replaces the granted file with the fixed 1,024-byte binary fixture and therefore requires write authority. These are native acceptance diagnostics, not additional advertised service-v1 operations.

## Read semantics and SDK ownership

The request contains workspace/resource references, an optional expected version, an offset and a requested length of 1–1,024 bytes. Offset plus length must fit the logical integer range, 0–2^53−1. Starting at EOF returns an empty range with the empty-range SHA-256 and `eof=true`; starting beyond EOF fails. A successful native call returns the full requested range up to EOF.

The first service request validates the stored checksum and returns the observed version, complete file size, range SHA-256 and current epoch. The SDK collects up to 40 bytes per subsequent exchange at that pinned version. Every chunk repeats the workspace identity and undergoes current authority and version checks. A change or revocation between chunks fails the whole logical read. An owner edit after the last admitted chunk does not make the earlier verified observation false.

There is no persistent read lease, server-side snapshot or retained read buffer. The file service owns temporary bounded buffers while handling each request; the SDK assembles into the caller's buffer. This trades repeated bounded reads/checks for simple lifetime and revocation rules. Existing staging and operation quotas are unchanged.

The public guest API is:

```rust
let references = files.references(workspace_id, object_id)?;
let mut bytes = [0; 1024];
let info = files.read_range(
    rustic_sdk::files::read::Request {
        workspace: references.workspace,
        resource: references.resource,
        expected_version: None,
        offset: 0,
        length: 1024,
    },
    &mut bytes,
)?;
let verified_bytes = &bytes[..info.length];
```

The output buffer must be at least the requested length and at most 1,024 bytes, even if the file may end earlier. The collector validates the complete range before returning `Info`: object, context, pinned version, size, exact chunk lengths, canonical padding and final SHA-256 must agree. Errors and abandoned collection clear the entire caller buffer. It allocates no second 1 KiB client buffer and performs no implicit retry.

The existing RPC owner authenticates the peer and correlation for each exchange; the new methods share that stream with legacy file calls. A maximum-size read uses one OPEN and 26 chunk exchanges. Waiting remains subject to the caller's `Progress` policy and the existing bounded RPC waits. Ctrl-C can leave the shell's wait without presenting partially collected bytes as a successful read. The hash uses the reviewed software SHA-256 dependency described in [the dependency inventory](dependencies.md); hashing does not authenticate a publisher or make a reference into a capability.

## Native wire binding

IPC retains its 64-byte payload. The existing file packet supplies native protocol version, opcode, status, payload count, object ID, argument, context generation and resource version. Logical service version 1 is separate from that native framing version.

| Operation | Request | Successful reply |
| --- | --- | --- |
| `REFERENCES` (15) | Object ID; selected workspace directory in `arg`; zero version/count | Same object/workspace, zero version, 16 payload bytes of nonzero lineage |
| `READ_OPEN` (16) | Object ID; requested length in `arg`; expected version or zero for null; 30-byte payload below | Object ID, full size in `arg`, observed nonzero version; 40-byte payload: range SHA-256 (32) and retry epoch (8) |
| `READ_CHUNK` (17) | Same identity payload; length 1–40; required nonzero pinned version | Object ID, full size, pinned version and returned byte count/data |

The 30-byte read-request payload contains lineage at 0–15, workspace directory ID at 16–19, absolute byte offset at 20–27 and logical service version at 28–29. Integers are little endian and the remaining ten bytes are zero. `REFERENCES` is a native bootstrap helper for an already authorized object; it is not live capability discovery.

Known read opcodes with another logical service version return `UnsupportedVersion`. A missing usable lineage/epoch returns `Unavailable`. Invalid ranges return native validation errors; scope failure, expiry or revocation remains a denial. Canonical error replies contain only opcode/status/context, with all result fields zero. Malformed framing, response mismatch, closure and interruption are local client failures, never successful logical results or invented remote responses.

The ABI owns identities and codecs; the SDK owns collection/progress; `rustic-file-service` owns read authority and hashing; `rustic-fs` owns namespace/version/data facts. The kernel imports none of those service implementations and gains no filesystem policy or SHA dependency.

## Reproduction and coverage boundaries

Run configured workspace checks and the focused pure codec/collector tests from the reference Ubuntu environment:

```sh
source ~/.cargo/env
cargo xtask check
cargo test -p rustic-abi -p rustic-sdk --test file_read --locked
```

After installing the pinned contract-validator environment using [the service guide](SERVICE-CONTRACTS.md), run the bounded host fixture separately from native execution:

```sh
.cache/contracts-venv/bin/python -m tools.contracts read-check \
  --output artifacts/read-host.json
.cache/contracts-venv/bin/python -m unittest discover -s tools/contracts/tests -v
python3 tools/terminal_test.py
.cache/contracts-venv/bin/python -m tools.contracts read-native \
  --evidence artifacts/terminal-test/terminal.json \
  --output artifacts/read-native.json
```

`read-check` identifies its backend as `bounded_host_read_fixture`: three immutable objects and one read operation. It does not execute a guest service, durable operation, model, provider adapter or MCP client. `read-native` validates the shared range exchanges collected from the real native SDK/UART workload and records the kernel hash. It does not boot a VM itself or independently replay every authority/recovery assertion in the terminal driver.

Both checks use the `complete_bounded_ranges` fixture profile in `contracts/services/v1/fixtures/read-ranges.json`. The native SDK completes a requested bounded range up to EOF. General service-v1 also permits a shorter non-EOF reply that makes progress; rejection of that reply by this stricter fixture profile is not evidence that the general schema forbids it. Coverage is the shared read-range subset, not full eight-operation conformance.

The native terminal driver exercises text, empty/binary files and range boundaries, C/H read authority without receipt inspection, owner edits between chunks, revocation, fresh grants after restart, workspace recreation, reclamation and surviving references in a separate VM boot. It records `read_contract` within `terminal.json` and preserves transcripts and independent disk evidence. Pure tests separately challenge malformed packets, output clearing and mixed/incomplete collection. Treat the commands and case inventory as acceptance procedures; passing results must be tied to the revision and evidence produced by the actual run.

The direct `terminal-test` scenario is also included in the complete boot suite and the isolated executor. For evidence from `python3 tools/boot.py run --mode terminal-test`, the direct result path is `artifacts/boot/terminal-test/terminal.json`; use the returned artifact path for an isolated job.

## Validated local increment — 2026-09-10

Configured formatting, Clippy and native builds passed with 109 Rust tests, 91 runner tests and 29 contract tests. The full 22-scenario direct suite passed; terminal acceptance completed 925 first-boot commands and a second persistence boot, while recovery completed nine groups across 18 boots. Polling makes command counts variable. The shared native checker accepts all 15 range cases on ELF `e4ee53c682ef965cb1f93a4276ce8d9d371354a95547b152b647a8f510112f76`. The separate [36-boot calibration and reviewed resource cost](MEASUREMENTS.md#native-read-calibration--2026-09-10) uses that same ELF.

Review included separate ABI/SDK, filesystem/service, dependency/conformance and cross-boundary checks by collaborating agents, plus automated validation. This is not an independent security or cryptographic audit. Final publication and isolated-executor evidence are linked from #1/#12/#13/#22/#43.
