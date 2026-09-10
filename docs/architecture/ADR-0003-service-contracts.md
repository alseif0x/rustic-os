<!-- SPDX-License-Identifier: Apache-2.0 -->

# ADR-0003: bounded service schemas and verifiable file operations

Status: accepted design baseline, 2026-09-10; native services are not implemented by this decision.
Issue: [#6](https://github.com/alseif0x/rustic-os/issues/6). Requirements: R02–R06, M1; boundaries: ADR-0001 and ADR-0002.

## Context and decision

RusticOS has an independent kernel, isolated processes, a native SDK, bounded IPC and an internal block driver. It needs service contracts that both applications and agents can use without interpreting the screen. MCP cannot define the kernel's authority or disk consistency. The immediate workload is a precreated file in a scoped workspace, not a universal workflow engine.

Adopt the eight logical operations and behavioral rules in [SERVICE-CONTRACTS](../SERVICE-CONTRACTS.md). Use modular JSON Schema 2020-12 files plus a small catalog as the canonical logical shape source; generate neutral tool descriptors from it. Keep resource authority and stateful semantics in their owning services. Keep a bounded native representation/encoding separate from this host/tool representation and require conformance when that binding is implemented.

Choose explicit bounded binary file data, expected versions, retained operation receipts, key lookup after a lost first reply, count-bounded retention with durable retry epochs, and an unknown outcome distinct from failure. Whole-file replace v1 is at most 1 KiB and publishes all or none; future streaming/multi-effect methods must define their own partial effects. Avoid freezing every product area's future API in H1.

No Rust crate, syscall number, Rust serialization dependency or kernel JSON parser is added. The useful compilation/trust boundary for shared native code will be extracted with #44/#12's actual encoding, rather than a second handwritten Rust model with no consumer. Pure schemas cannot represent transferred kernel authority, prove authorization or establish crash consistency.

## Alternatives and costs

| Reference / alternative | Relevant mechanism and RusticOS choice |
| --- | --- |
| [Windows MIDL/RPC](https://learn.microsoft.com/en-us/windows/win32/rpc/the-idl-and-acf-files) | Separates interface description from configuration and implementation. Adopt explicit shared contracts and derived client metadata; importing the Windows RPC runtime is not appropriate for the current bare Rust environment. |
| [Linux desktop D-Bus specification](https://dbus.freedesktop.org/doc/dbus-specification.html) | Typed method calls, replies, errors, signals and introspection provide a useful service-interface reference. D-Bus is a userspace service mechanism, not the Linux syscall ABI; adopting its daemon/wire format would add a separate runtime/port. Use the ideas without claiming D-Bus compatibility. |
| [macOS XPC services](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingXPCServices.html) | Service separation and connection/error lifecycles are useful references. Apple-specific runtime and service management are not available in RusticOS. Preserve request/reply and interruption semantics with RusticOS-owned processes and handles. |
| [Fuchsia FIDL](https://fuchsia.dev/fuchsia-src/development/languages/fidl/tutorials/fidl) | A typed interface language can generate bindings. A full IDL compiler/runtime is a possible later investment; for eight operations it adds more infrastructure than needed before a native transfer boundary exists. |
| [JSON Schema 2020-12](https://json-schema.org/draft/2020-12/json-schema-validation) | Suitable for logical/tool shapes and existing host validators. Adopt it with strict message-local checks; character lengths, content annotations and schemas alone do not enforce decoded bytes, authority or state transitions. |
| Handwritten descriptors or MCP-only contracts | Easy to begin, but duplicates schemas or couples the product to an adapter. Generate descriptors from the shared source; #43 compares actual integration cost and #39 owns MCP compatibility. |

These are design comparisons, not claims that RusticOS improves on those systems. No third-party source was copied. The original host tool uses [jsonschema's offline registry API](https://python-jsonschema.readthedocs.io/en/stable/referencing/) with exact hashed wheels recorded in [the dependency inventory](../dependencies.md). The protocol uses [RFC 4648 canonical base64](https://www.rfc-editor.org/rfc/rfc4648) only at the JSON boundary.

For #44, a bounded copied-buffer call is the first candidate against chunked IPC: one sector exceeds the current payload, and neither extra exchanges nor longer kernel execution is free. Exact encoding, lifetime and fairness tests remain #44's responsibility. This decision does not silently widen IPC or expose DMA.

## Validation and review conditions

Nine schema files, eight exported descriptors, 62 positive/negative messages, nine exchanges and 14 host tests establish shape, bounded data and message-local consistency. They do not establish a stateful server, actual cancellation, grants, file persistence or MCP interoperability. [Stateful conformance scenarios](service-contract-cases.md) assign those tests to #43/#44/#12/#13/#22. The earlier finite operation model is independent evidence about assumed state ordering.

Revisit the 1 KiB mission limit when a measured native transfer exists; revisit schema generation when a second real binding reveals duplication; revisit receipt storage if the selected filesystem cannot recover data/version/receipt together. In that case revise the contract or keep replace unavailable until recovery is implemented—never downgrade a successful result silently. Revisit retained-record capacity after #20 measurements. Do not expand into arbitrary delegation, automatic workflow rollback, universal application semantics or mandatory permission tiers without a concrete workload and explicit evidence.

## Implementation follow-through — 2026-09-10

#44 selected and implemented the copied asynchronous sector boundary in [BLOCK-ACCESS.md](../BLOCK-ACCESS.md), with actual ring 3 persistence, cancellation, scope and lifetime tests. The proposal above records the decision context at #6; the block document now owns its exact wire contract. File/receipt semantics and stateful backend conformance remain #12/#13/#22/#43.
