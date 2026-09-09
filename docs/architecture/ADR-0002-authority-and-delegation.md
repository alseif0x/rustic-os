<!-- SPDX-License-Identifier: Apache-2.0 -->

# ADR-0002: minimum shared authority for useful service work

Date: 2026-09-10. Status: adopted design baseline for #5 under the owner's instruction to proceed and reassess the approach. **This decision is not implemented product authorization.** Requirements R02/R03/R04/R11. Implementation: #6, #44, #12, #13 and later domain services. [Decision cases](authority-cases.md).

## Problem and decision

RusticOS needs one authority model for ordinary applications, human-operated clients and agents. Its first useful proof is a client working on workspace A, being denied access to B, and losing its access on owner revocation while manual control remains usable. A model and MCP are not needed for that proof.

Drop **Low/Medium/Total as mandatory permission levels**. They were planning shorthand, not implemented ABI values. “Medium” does not identify writable resources; “Total” does not identify a trust domain. Replacing them with Observe/Work/Admin would still prioritize labels over behavior. No replacement set of fixed tiers or mandatory UI templates is adopted. Templates may be added later if they simplify a demonstrated workflow, while exposing the actual grants.

Build on kernel-owned handles and authenticated process identity. Services enforce explicit resource/action grants for a trusted session context. The supervisor provisions the context and its client instances; it does not become a proxy for every file byte or duplicate every service's policy. Start with owner-issued session grants and explicitly bounded helper grants, not an unrestricted hierarchy of agents delegating to other agents.

Keep **authority, autonomy and confirmation policy separate**. A session can run automatically with read-only access, or automatically with broad owner-delegated administration. The same underlying permissions apply to manual and programmatic calls. Automatic mode and model confidence grant nothing; a confirmation can only permit an already authorized operation to proceed under its valid preconditions.

## Why this approach

| Alternative | Benefit | Cost or gap | Choice |
| --- | --- | --- | --- |
| Broad account permissions plus prompts around agent tools | Familiar and quick to demonstrate | Other client paths/helpers can bypass tool-level checks; prompts do not specify durable resource scope | Do not make the adapter the security boundary |
| Fixed levels or three renamed templates | Simple labels | Conceal combinations such as read A, write B, use one credential but never export it | Remove as architecture requirements |
| General capability/delegation framework before files | Expressive, potentially reusable | Adds persistence, delegation graphs, invalidation and user experience decisions before a real service can test them | Defer generality; keep the minimum contract and failure obligations |
| Handles plus service-owned scope checks and bounded session grants | Fits current IPC mechanisms and the planned user-mode file service; same behavior across clients | Still requires real identity binding, revocation and service tests; handles alone are insufficient | Selected for H1 |

Windows [access tokens](https://learn.microsoft.com/en-us/windows/win32/secauthz/access-tokens) provide a trusted process/thread security context, and [AppContainer](https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation) illustrates mediated resource access. Linux [capabilities](https://man7.org/linux/man-pages/man7/capabilities.7.html) divide superuser privileges into per-thread privileges, but those bits are not a workspace-scoped object model. macOS [selected-file read access](https://developer.apple.com/documentation/BundleResources/Entitlements/com.apple.security.files.user-selected.read-only) is a useful resource-selection precedent. These mechanisms inform the design; no compatible implementation or whole-platform superiority is claimed.

seL4's [capability API](https://docs.sel4.systems/projects/sel4/api-doc.html) distinguishes capability derivation/revocation. The lesson for future expansion is to retain delegation relationships, not assume a copied/moved handle has solved them. RusticOS does not acquire seL4's mechanisms or proofs. No third-party implementation or dependency is added.

## Minimum authority record

A grant is trusted state, not a model-authored JSON object. It identifies:

- **Subject:** authenticated current session and process/service instance, including its lifetime. A manifest name, a PID from an old boot, or an argument claiming owner identity is insufficient.
- **Resource:** a stable service-owned object/container or explicitly selected namespace. File containment must survive resolution, links, renames and identifier reuse; a textual prefix is not the boundary.
- **Actions and constraints:** for example read, replace, launch an identified utility, bounded block access, send selected data to one destination, use a credential or activate a candidate. These are separate actions, not a numeric level.
- **Lifetime:** session end or an explicit current-boot monotonic deadline. At the deadline the grant is expired. Unlimited duration requires explicit issuance; omission does not mean unlimited.
- **Helper authority:** whether a helper may receive a specified subset, bound to the same revocable session context. It cannot gain scope, outlive that context or multiply its resource budget.

The initial implementation may use a bounded table. #6/#13 choose and measure encodings/capacities; this ADR does not invent syscall numbers, cryptographic tokens or an extensible policy language. No matching grant means denial. A supported operation must be covered as a whole; combining one grant's resource with another's action cannot manufacture a permission. New methods do not silently expand an enumerated action set.

The trusted owner path can grant more authority. An already authorized administrator may manage grants within its explicit ceiling, without redundant root-owner prompts. A workspace administrator cannot expand that ceiling to the guest or host. H1 does not expose arbitrary recursive client-to-client delegation; broader delegation needs its own bounded implementation and tests.

## Resource boundaries, now and later

Every row uses the subject and lifetime above. The later rows constrain future contracts; they are not prerequisites to running the first file mission.

| Resource | Distinct rights / scope | Enforcer and minimum failure obligation |
| --- | --- | --- |
| Files and directories | Read/create/replace/rename/delete in selected objects or workspace | File service; reject B, stale versions and escaping links/renames |
| Processes and helpers | Launch selected executable, inspect/stop owned work, provision subset grants | Supervisor/kernel; a child cannot gain authority from a filename, command line or parent label |
| Block device | Read/write/flush selected device/range, designated file-server instance | Kernel boundary in #44; ordinary file clients never inherit the server's raw disk access |
| Devices and capture | Selected sensor/device/surface and permitted operation | Owning service; capture/input cannot cross the authorized selection |
| Networking and selected-data egress | Connect/listen/send and destination/protocol/port; source selection for brokered model calls | Network/egress service; reading a file does not grant permission to transmit it or follow a redirect to another destination |
| Credentials | Use for an operation/destination; export/manage separately | Credential broker; use-only never returns the secret in model buffers or logs |
| Services, configuration and grants | Administer named resources and a defined grant ceiling | Supervisor/domain service; no implicit administration of unrelated resources |
| Builds and activation | Read source, create artifact, build, verify, activate are separate rights | Workspace/build/activation services; compiler success or source-write access cannot authorize installation |
| Evidence and control | Query selected operation/receipt, cancel work, revoke session | Owning service and owner control; a log is not an authoritative outcome and cancellation is not undo |

A forwarded request must preserve a trusted requester context. A service uses its own mechanism access only to fulfill the client's authorized operation. It must not lend its broader rights to arbitrary caller arguments. The current SDK manifest describes identity/features for admission and grants no resource authority. Current IPC ownership/move checks remain valid lower-level mechanisms, not completed product authorization.

## Revocation that we can actually demonstrate

For H1, a session context is the revocation root for its client and explicitly provisioned helpers. Resource services must know which contexts they accept. Moving an endpoint cannot detach access from that context. A new independent background service needs explicit supervisor/owner provisioning.

At each bounded effect boundary, the owning service checks live context/lifetime and resource preconditions together with admitting the effect in serialized local state. Checking permission only in the adapter, or checking and later writing without synchronization, is insufficient. #13 chooses the concrete generation/handle-invalidation mechanism; an old context must not revive when the owner grants fresh access.

Revocation has distinguishable stages: requested, access fenced, and outstanding effects settled. The supervisor stops fresh issuance and obtains acknowledgment from the relevant services after they reject old queued/future work. Already submitted I/O may finish. A timeout or killing the origin client is not proof that a server or device stopped. Preserve progress for owner/control traffic under pressure.

In the first mission this protocol covers the file service, client/helpers and their I/O route, not every hypothetical future subsystem. If a required service cannot acknowledge or its effects cannot be reconciled, report incomplete takeover and the outstanding state. Manual inspection and independent owner actions stay available. Full takeover completion cannot be claimed while relevant effects remain unknown.

Already committed edits are not undone. A stale file version must conflict rather than overwrite a human edit. A lost response requires result lookup/reconciliation before retry; a local receipt design cannot promise exactly-once external effects. These obligations extend to each new service when it is introduced.

## Owner, restart and the trust boundary

R0 trusts the host/hypervisor, boot chain, kernel and kernel drivers. The supervisor and relevant effect services are also trusted for their policy boundaries. An untrusted agent, application, document, model response or compiler must not become one of these components by naming itself as trusted.

Before #13, a fixed trusted launcher may provision the designated #44/#12 test service. This is fixture bootstrap, not a secure owner login. #13 must establish a supervisor-owned control path whose input/approval cannot be forged by ordinary client output or messages. Recovery must work without the agent, GUI or network. General remote/multi-user authentication is not silently provided by that first path.

Active contexts, handles and elapsed-time leases do not survive process/guest incarnation changes. Persisted owner policy may authorize **fresh** issuance for a selected launch; it cannot restore old handles or revive revoked access. H1 may disable automatic reissuance and require the owner-control path after boot. Any implementation enabling it must persist revocations consistently and test interrupted policy writes before claiming that behavior. Unreadable/inconsistent policy fails into manual recovery with delegated auto-start disabled. R0 does not provide hostile-host or offline rollback protection.

Full control remains an explicit delegation over a named domain, including the security foundation when selected. Guest control never grants control of host processes, physical disks or personal folders. A host build bridge applies its own independent authorization.

Some combinations cannot be enforced: raw access to storage can bypass file exclusions; arbitrary kernel/policy replacement can defeat in-guest checks; plaintext plus unrestricted outbound channels defeats selected-only egress. The trusted grant path must identify such conflicts and offer narrower mediated access or a clearly unrestricted delegation. Do not promise impossible exclusions or forbid full owner delegation on principle. If the owner delegates the foundation, recovery/evidence required to survive that delegate must be outside its authority.

The initial provider path should mediate selected content and credential use without giving the agent bypass sockets or secret material. This is not general information-flow control: arbitrary programs with both plaintext and unrestricted send can transmit it. Data already released cannot be recalled by revocation.

## Delivery gate and deferred work

First demonstrate A1 with a deterministic native client: read/modify A, deny B, preserve an intervening human edit, revoke client/helper access, expose any already committed effect and retain manual progress. Repeat file persistence after reboot with fresh authority. No model, fixed tier UI or MCP is needed to validate that foundation.

#6 specifies the smallest shared contracts for this mission and the block/file interface. #44 enables controlled user-mode block access; #12 adds real files; #13 implements the initial authority boundary and control path. #24 later connects mode/confirmation behavior to it. Subsequent network, agent, MCP and build issues repeat the applicable cases on their actual routes.

Defer configurable tier/template UX, a general grant language, unbounded delegation graphs, offline signed grants, system-wide information-flow tracking and a generalized durable task engine. They may be useful, but must earn their cost through a concrete workload. This narrows implementation order without removing v0.1's required manual use, optional agent, owner control or recovery outcomes.

## Decision validation and review conditions

The [20 case walkthroughs](authority-cases.md) specify expected decisions and their future runtime owners, including six client paths. They are design review evidence, not executed guest authorization tests. The existing [finite model](operation-model.md) illustrates local version/revocation races; it does not validate this policy or a cross-service protocol.

Review by the implementing agent; no independent security audit. Publication/verification evidence is recorded in #5. Revisit if the first file mission cannot meet these rules with bounded service state, if invalidation starves manual control, or before enabling unrestricted delegation, shared memory/DMA or offline/remote authority. Do not widen the abstraction merely to satisfy an old issue checklist.

Earlier Low/Medium/Total wording is superseded at the owner's request on 2026-09-10. No replacement three-tier system is selected. The next implementation work is the shared contract and user-mode storage/file path for that first useful workflow.
