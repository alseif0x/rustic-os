<!-- SPDX-License-Identifier: Apache-2.0 -->

# Agent integration: native contracts and adapters

Date: 2026-09-09; updated 2026-09-10. Status: integration proposal with the first logical contracts now specified by #6/[ADR-0003](ADR-0003-service-contracts.md). Stateful backend/adapter measurements remain #43. Kernel IPC is implemented in #34; service APIs and adapters are not implemented by this document.

## Conclusion and scope

RusticOS should provide structured services to people, applications and agents. MCP will be an interoperability adapter, not a mandatory protocol for the kernel, applications or integrated agent. The product advantage is discovering real capabilities, operating on resources and verifying effects without interpreting pixels. An HTTP API for every syscall does not achieve that goal.

Layer separation existed in #1/#6/#22/#23/#39, but concrete methods and a comparison of paths were missing. Also, #39 required HTTPS even when a local client could use stdio. This review makes the work concrete and corrects that dependency.

## Proposed layers

| Layer | Responsibility | Proposed choice |
| --- | --- | --- |
| Kernel/IPC (#3/#34) | Isolation, handles with rights, channels, waits, limits | Explicit ABI and bounded messages; decide encoding and handle transfer through portability tests. |
| Services/SDK (#6/#11/#13) | Resources, typed methods, states, events and effective authority | Versioned contracts; one service implementation shared by CLI, GUI and tools. |
| Tools (#22) | Agent-understandable actions, contextual selection and verifiable results | Native catalog with structured inputs/outputs and reviewed contract adaptations. |
| Integrated agent (#23) | Observation loop, model calls, authorized execution and verification | Execute structured model calls locally through the native catalog. |
| External clients (#39) | Interoperability with agent hosts | MCP version and transport tested against an independent client. |
| Additional integrations | Conventional clients or delegation to independent agents | Evaluate HTTP/OpenAPI, gRPC or A2A only when there is a consumer and measurable need. |

The registry enables service discovery, but need not become a proxy for every OS byte. Separate control (commands, permissions, state) from bulk data (files, images, video): use authorized, bounded streams or handles instead of copying everything into the model context.

Fuchsia illustrates separation between typed definitions, bindings and IPC channels; it informs the design without implying a full FIDL port. D-Bus contributes introspection, property, event and versioning patterns. [FIDL](https://fuchsia.dev/fuchsia-src/concepts/fidl/overview), [D-Bus API design](https://dbus.freedesktop.org/doc/dbus-api-design.html).

gRPC provides service contracts and unary/streaming calls. It is an alternative to investigate for clients that require it; usefulness does not establish that its runtime is the best foundation for a new OS. [gRPC concepts](https://grpc.io/docs/what-is-grpc/core-concepts/).

## How the agent connects

1. The integrated agent obtains tools available and permitted for its session.
2. It sends the model only relevant descriptors and authorized context.
3. The model returns a name and structured arguments.
4. The local executor validates schema, limits and delegation; the service checks authority again when acting.
5. The service returns a result or operation identifier.
6. The agent inspects state/evidence and chooses the next step within its budget.

This flow allows remote inference while the executor lives inside RusticOS; it does not require exposing an inbound OS server. This architectural choice follows the documented function-calling flow in which the application executes code. [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling).

An external client can use MCP to reach the same catalog. The consulted specification distinguishes stdio and Streamable HTTP. Choosing stdio does not itself solve host/guest crossing: if the client runs outside the VM, a defined, authorized bridge is required. Remote HTTP adds #16/#17, authentication and exposure scope. Do not create custom transports without a demonstrated need. [MCP 2026-07-28 transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports).

Pin the protocol version and test actual SDK/client compatibility rather than automatically following latest. Provider-managed MCP connections may support different transports from a local host. [OpenAI MCP](https://developers.openai.com/api/docs/guides/tools-connectors-mcp).

A2A addresses collaboration among independent agents and their tasks; it does not replace file, process or window APIs. It remains a future option, without a new v0.1 dependency. [A2A and MCP](https://a2a-protocol.org/dev/topics/a2a-and-mcp/).

## First specified vertical contract

[Shared service contracts v1](../SERVICE-CONTRACTS.md) replaces the earlier provisional argument names and example. The canonical catalog retains capabilities.list, capabilities.describe, files.read, files.replace, operations.get, operations.cancel, events.read and system.status. It specifies bounded binary data, typed errors, current-authority/version checks, receipts, cancellation, retry epochs and lookup when the first response is lost. Generated descriptors use these same schemas.

The host suite validates message shapes and correlated examples; it does not implement a backend or prove guest effects. #43 compares adapters over a stateful backend. #44 implements [native user-mode block access](../BLOCK-ACCESS.md); #12 adds files, #13 authority and #22 the complete guest mission. The native product path need not wait for the host comparison to begin storage work.

## Expansion by product area

| Family | Operations to design | Owning issues |
| --- | --- | --- |
| Files/workspaces | list, create, move, delete, patch and versions | #6/#12/#25 |
| Processes and services | list, start, inspect, stop; health and restart | #6/#10/#13 |
| Configuration and adaptation | inspect, validate, preview and apply; explain degradation | #6/#38 |
| Desktop/applications | list windows, activate, semantic actions, accessibility and selected capture | #40 |
| Browser | navigate, inspect selected content, identify elements and act with verifiable state | #41 |
| Builds/candidates | submit bounded job, follow tests, verify and activate artifact | #42/#26/#27 |

First-party app coverage includes product actions and semantic state. External apps only have the coverage supported by their API/adapter/accessibility; declare where vision is needed. Complete semantic control over every external binary is not a realistic promise.

## Decision experiment (#43)

Compare the same contract and fixtures through:

- A reference native client.
- A function-calling adapter with simulated model responses.
- A minimal host MCP adapter with an independent client.

The adapter prototype is disposable and bounded; it does not require all RusticOS services or complete #23/#39. Keep payload, backend, hardware, versions and load fixed. Measure p50/p95 latency, RAM, bytes per mission, call count and integration cost. Separate transport time from inference time; simulation does not measure the latter.

Required tests: successful mission, denial, revocation, invalid arguments, concurrent conflict, retry after lost response, cancellation with partial effects, pagination and expired cursor. Reject any path that changes permissions or claims false success, even if it is faster.

Record IPC/encoding candidates and runtime requirements; do not extrapolate host measurements to the VM. Set acceptance budgets from the baseline before selecting an implementation. Repeat cases in the guest when closing #22/#39. Real-model tool-selection quality is evaluated in #23, not with simulated responses.

## Realistic improvements

- Executable specification before multiplying endpoints: discover → read → modify with a precondition → verify, including under failure.
- Queryable capability map explaining why a feature is absent, degraded or unauthorized when policy allows disclosure.
- Product actions and an application semantic tree as part of the SDK; GUI and agent act on the same state.
- Change previews for configuration and candidates where feasible; do not invent a perfect dry-run for irreversible operations.
- Structured receipts and reproducible mission records with sensitive data omitted; expand to replay in disposable environments later.
- Defer A2A and additional HTTP/gRPC adapters until a real consumer exists. Fewer initial protocols preserve resources for the kernel, drivers, SDK and browser.

## Pending decisions

#3/#34 assign IPC, ABI and encoding with measurements and portability; the initial kernel implementation is now documented in [IPC.md](../IPC.md). #6 now specifies the initial schemas and service semantics in [SERVICE-CONTRACTS](../SERVICE-CONTRACTS.md); new product operations are specified with their actual services. #39 owns client/SDK pairing, MCP revision and transport. #43 owns comparative evidence. No library or transport is yet claimed to be best for the product integration: this proposal establishes the boundaries and how to decide.

The [systems roadmap review](systems-roadmap.md), dated 2026-09-10, extends this proposal with stage gates, explicit block-to-user access (#44), service-capacity planning and bounded experiments for task continuity. Its [finite operation model](operation-model.md) illustrates why commit-time resource and authority checks and a recovery contract matter. The research model itself does not resolve #5/#6 or implement the #43 adapter comparison; #5/#6 are now separately accepted decisions in ADR-0002/ADR-0003.
