<!-- SPDX-License-Identifier: Apache-2.0 -->

# Authority decision cases

Date: 2026-09-10. Reviewed specification for [ADR-0002](ADR-0002-authority-and-delegation.md) and #5. **None of these rows claims an executed guest authorization test.** They are worked decisions whose runtime tests belong to the listed implementations. There is no second policy engine or simulated server to mistake for OS enforcement.

## Common fixture

One owner provisions a current session for client C: read/replace in workspace A, explicitly selected utility execution, no B access, no raw device/network/credential/activation rights. A helper H receives only read access to A within the same session. Each subject is a live authenticated instance; file A starts at version v1. The owner has a separate recovery/control context. Tests use disposable data only. Additional grants below are explicit changes to this fixture, never inferred from a mode or label.

| ID | Request or event | Reviewed expected result and reason | Runtime owner |
| --- | --- | --- | --- |
| A01 | C reads A | Allow within its live resource/action grant | #12/#13 |
| A02 | C replaces A with expected v1 | Allow one defined change and return an inspectable result | #12/#13/#22 |
| A03 | C reads or writes B through a supplied ID/path/link | Deny; resource naming does not confer authority or escape A | #12/#13 |
| A04 | Human edits A to v2 before C commits against v1 | Conflict, preserving the human edit; an old observation cannot authorize overwrite | #12/#25 |
| A05 | H reads A, then tries to replace it or launch another helper | Read allowed; both other operations denied because no such grants exist | #13 |
| A06 | C changes its manifest/name or sends owner identity in arguments | No elevation; use authenticated instance and trusted context | #13/#44 |
| A07 | C asks the broadly provisioned file server for B/raw sectors | Deny; the server cannot lend its own mechanism access | #44/#12/#13 |
| A08 | Owner revokes the session with queued C/H work | Reject old future units after the acknowledged fence; report already admitted I/O separately | #13/#24 |
| A09 | Owner grants C fresh access after A08; an old queued request arrives | Old context remains invalid; a fresh grant never revives it | #13 |
| A10 | A lease expires, or client/service/guest restarts | Reject the expired/stale context; require valid fresh issuance, not a restored handle | #13 |
| A11 | Response is lost after a permitted file change | Inspect an authorized result/reconcile before retry; no unsupported claim of failure or duplicate success | #6/#22/#43 |
| A12 | A service does not acknowledge revocation or device work remains pending | Takeover remains incomplete with outstanding state visible; owner control still progresses | #13/#20/#24 |
| A13 | Automatic mode with only read A granted requests replacement | Deny; automatic mode is not write authority | #24 |
| A14 | Automatic mode with explicitly granted administration acts inside its scope | Allow according to its confirmation policy, without redundant prompts; outside scope still denied | #24/#27 |
| A15 | A user confirms an otherwise ungranted action, or confirms v1 after v2 exists | Confirmation alone grants nothing; grant expansion is separate and stale preconditions still fail | #13/#24 |
| A16 | C reads A and tries to send it to a model without egress authority | Deny transmission; read and destination permission are distinct | #16/#23 |
| A17 | Add selected-A egress to provider P and use-only credential K; request P, then provider Q or export K | P may receive the selected authorized data; Q and secret export are denied | #16/#17/#23 |
| A18 | Compiler reads authorized source, builds an artifact, then requests activation/host access | Build under independent runner limits; activation and host access require their own authority | #26/#27/#42 |
| A19 | Owner delegates full control of A or the guest, then delegate requests host resources | Allow supported in-domain operations; guest authority cannot grant host access | #24/#27/#42 |
| A20 | Owner requests protected-file exclusions plus raw disk/kernel control that bypasses them | Report the incompatible guarantee; allow narrower mediation or explicit unrestricted delegation with external recovery limits | #13/#27/#28 |

## Same behavior across client paths

Repeat A01–A04, A06, A08–A11 and A13 using each supported path below with **the same effective subject/resource grants**. These are future conformance obligations, not 60 or more tests already passed.

| Path | Binding that must reach the enforcing service | Implementation |
| --- | --- | --- |
| Console | Shell's authenticated session and resource context; no privileged shortcut for ordinary commands | #14 |
| Native API | SDK handle plus kernel-provided current sender; forged context fields rejected | #11/#13 |
| Subprocess/helper | Supervisor-provisioned subset in the same revocable session | #13 |
| Tool adapter | Validated arguments forwarded under the original context; schema validity grants nothing | #22 |
| MCP client | Authenticated external session mapped to scoped local authority; no serialized handle accepted as a grant | #39 |
| Future compiler/build client | Explicit source/output grants and separately authorized bridge/runner identity | #26/#42 |

The trusted owner-control path is a different authority context from an ordinary console command. Its ability to issue a grant must never be inferred merely from which UI sent a request.

## Review outcome and scope

The walkthroughs cover subject, action, resource, lifetime, helper scope, indirect access, mode independence, revocation/regrant, partial/unknown effects, egress, credentials, activation and full-control limits. Allowed and denied outcomes follow the explicit fixture; no Low/Medium/Total or replacement tier is needed.

Current kernel/IPC/SDK/block evidence remains evidence only for those implemented mechanisms. The earlier finite model covers different, deliberately limited state transitions. #13 and each later service must provide actual guest/transport evidence before their implementation issues close. Review was by the implementing agent, without independent audit.

## Native implementation evidence — 2026-09-10

The [native C/H mission](../AUTHORITY.md) now exercises A01–A05 for the bounded file surface, A06/A07 for privileged control and the file service, A08 for queued requests/staging and shared fencing, A10 for inherited expiry/root death/service restart, and A12 for settled I/O failures that require recovery. It also moves a real kernel handle and checks identity and revocation after movement. The separate recovery suite supplies A11. Portable tests exercise regrant rejection under A09; full native service-v1 regrant/reference integration remains open. This evidence does not close the entire matrix: links are unsupported, a missing acknowledgment or still-pending device effect is not yet covered, and A13–A20 retain their listed service owners.
