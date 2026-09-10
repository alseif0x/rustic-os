<!-- SPDX-License-Identifier: Apache-2.0 -->

# Requirements and reference platform — experimental v0.1

Status: working baseline adopted for #2 on 2026-09-09. The owner authorized proceeding with the first-delivery proposal (“go ahead”). The previously agreed scope is preserved; technical choices are made within that delegation. Acceptance of the document does not establish implementation or prior individual approval of every parameter.

## Product and limits

RusticOS is an independent OS written primarily in Rust. It must work manually without AI and let agents operate first-party capabilities through APIs/tools, with complementary visual access. The integrated agent runs inside RusticOS; inference, compilation and test VMs may be external, with their location declared. Console, GUI and agent share services and authority.

v0.1 includes boot, isolated processes, persistent files, shell, native tools, deterministic adaptation, networking/HTTPS, an optional agent, MCP interoperability, desktop and browser with local rendering, verifiable candidates and recovery. It does not promise production readiness, the entire web, every device, universal binary compatibility, local inference, complete internal compilation or hot kernel replacement.

Universality is measured separately across CPU architecture, devices, applications and interaction. Initially there is one CPU and a virtual device set. Adaptation means querying actual capabilities and selecting/degrading features according to policies and budgets; it does not mean automatically changing code to improvise support.

## R0 platform

The following are test-configuration parameters, not minimum requirements for a finished product.

| Element | Adopted baseline |
| --- | --- |
| Development | Ubuntu 24.04 in this machine's WSL2 (observed: 24.04.4 LTS). Linux CI on Ubuntu 24.04; image/tools assigned to #4. |
| CPU/guest | x86_64, QEMU qemu64 model, 1 vCPU. Do not use cpu=host as the reproducible baseline. |
| Machine | QEMU q35 family; #4 pins the pc-q35-X.Y name available in the selected version. |
| Execution | TCG required for the baseline test, without requiring nested virtualization. Additional acceleration has separate results. |
| Firmware | UEFI through OVMF, Secure Boot disabled in R0; disposable variable copy per run. #4 pins the build and hashes. |
| Boot | Limine protocol, x86_64-unknown-none ELF kernel; image with FAT/EFI boot volume and loader. #4 pins loader/protocol/bindings/image; #8 validates the path. |
| Memory | H0: 256 MiB. Initial integrated test: 2048 MiB. Restricted #20/#38 profile: 512 MiB. A browser is not required in every profile; declare degradation. |
| Disk | Read-only boot where tooling permits; separate persistence: virtio-blk-pci, 4 GiB virtual disk copied per scenario. Never physical host disks. |
| NIC | virtio-net-pci, disabled in H0/H1. From H3, isolated test networking; egress explicitly enabled per scenario. |
| Graphics | Loader-provided framebuffer with standard virtual VGA, target mode 1024×768; initial software rendering. No mandatory 3D acceleration. |
| Input | Emulated PS/2 keyboard and mouse; serial console for minimal manual interaction. |
| Clock/interrupts | Initially assigned to #33 under single-CPU R0, without an initial SMP requirement. #33 adopts PIC/PIT; LAPIC/IOAPIC remain future work. |
| Diagnostics | COM1 serial UART to log; QEMU monitor separate from the user channel. Test output distinguishable from boot text. |
| Entropy | virtio-rng-pci for #17; declared external source, without inferring entropy from the clock. |
| Sharing | No personal folders, clipboard or physical devices shared by default. Dedicated build bridge in #42. |

#4 must record exact Rust/Cargo, QEMU, OVMF, Limine, image-utility and CI versions, hashes/sources and verified commands. Do not download latest on every build. #4 cannot close with an unversioned checklist. This assignment avoids inventing versions before testing compatibility. Current versions are recorded in [the development guide](DEVELOPMENT.md) and tools/environment.toml.

The current memory implementation rejects usable physical regions above 1 GiB. The integrated 2048 MiB profile remains a future acceptance target and requires extending that implementation; current 256 MiB evidence does not establish it.

## Traceable requirements

Each scenario runs against the guest unless explicitly stated otherwise. Common evidence: commit, R0 configuration, tools, commands, expected/observed result, logs and artifact hashes. Preserve negatives as well as the successful case.

| ID | Verifiable requirement | Scenario / evidence | Issues |
| --- | --- | --- | --- |
| R01 | Original image and success/failure/hang diagnostics | B0/B1/B2: serial, exit/status and runner timeout; clean rebuild | #4 #8 #21 |
| R02 | Manual operation without AI, isolated processes and persistence | S1: process A fails without killing B/shell; save/reboot/read a file | #9 #10 #11 #12 #13 #14 #34 #44 |
| R03 | Structured coverage of included first-party capabilities | M1 and product-action catalog with API/tool and verification; coverage calculated over a versioned list | #6 #22 #40 #41 #29 |
| R04 | Owner authority separate from autonomy | A1: denied, revoked and workspace-scoped permission; takeover without a model | #5 #13 #24 #28 |
| R05 | Real optional agent operating without vision | M2, service result, model/configuration identity and shutdown | #23 #29 |
| R06 | Independent MCP client over real services | C1: repeat M1 through the adapter; same effects and denials | #39 |
| R07 | Networking, DNS and HTTPS inside the guest | N1: resolve/query fixture, reject invalid certificate and explicit DNS failure | #16 #17 #36 |
| R08 | Browser engine and rendering run locally | W1–W5, screenshots/state/actions and web-service tests | #7 #19 |
| R09 | Usable desktop and bounded optional vision | D1: open/activate/close window through API; capture authorized selection; return to console on failure | #18 #37 #40 #41 |
| R10 | Observable deterministic adaptation | P1: same input/policy yields same profile; limited memory exposes missing/degraded capabilities | #20 #38 |
| R11 | Verifiable change, candidate and recovery | M3, source/artifact hashes, tests and recovery boot without AI | #25 #26 #27 #42 |
| R12 | Reproducible publication and provenance | L1: inventory, license, experimental release, documented commands and limits | #30 #32 |

## Initial scenarios B0–B2 and S1

B0: build an image from a clean checkout and boot without networking/GUI/AI. Emit a build identifier and final success marker; the runner verifies both and the defined exit status. An early message is insufficient.

B1: test variant with deliberate panic. It must preserve diagnostics and end as failure even after the initial message. B2: blocked variant; the runner terminates only that VM at the deadline and returns timeout, not success. #8/#21 fix codes and timeouts after measuring the baseline. Repeat construction and compare hashes; do not promise binary identity before resolving differences.

S1, H1: boot the native shell without networking/GUI/model; launch two processes, trigger an invalid access in one and keep the other operational. Create/read a file, reboot and verify persistence. Manual serial input is allowed.

## Three product missions

M1, H2: precreated workspace and file. Discover tools, read content/version, replace using expected_version and an idempotency key, inspect the operation and verify content/hash. Repeat with denied permission, stale version and lost response; do not alter another workspace or duplicate effects. Deterministic client, screenshots/OCR/clicks disabled.

M2, H3: same objective with an agent running inside RusticOS and a real replaceable model, local or remote. Record steps, errors, budget and service verification. Inject an invalid call, provider outage and cancellation; do not expand permissions and preserve manual use. MCP is validated through C1 without forcing the integrated agent to use it.

M3, H5: modify a utility in a workspace, send a build to the bounded executor, receive an identified artifact and results, then activate under owner policy. Induce a failure and recover the prior version without AI or an available provider. Do not let agent text replace tests or hashes.

## Minimum web contract

Versioned local fixtures served by a controlled test server; rendering and JavaScript execute in RusticOS. #7 evaluates the engine and records gaps; a changing public website is not required as a test.

| Case | Fixture and acceptance |
| --- | --- |
| W1 HTML/CSS | Document with headings, paragraphs, links, list, local image and boxes with size/color/margin. Expected DOM and geometry; screenshot review with documented font tolerances. |
| W2 Unicode | UTF-8: Spanish accents and ñ, Greek and CJK text with licensed test fonts. Extracted content preserves codepoints; no unexpected replacement characters. |
| W3 JavaScript | Button increments a counter and updates the DOM; user and API actions produce the same observable value. |
| W4 Forms | Labeled fields, keyboard focus, GET/POST submission to fixture; server receives expected names/values and browser displays the response. |
| W5 HTTPS | Explicitly installed test CA, valid certificate accepted; expired, wrong-name or untrusted-CA certificates rejected. No silent bypass of validation. |

Initial accessibility: keyboard navigation, visible focus, queryable names/roles/state for first-party and fixture controls, textual errors. Comprehensive accessibility conformance and the entire web platform are not promised. Native forms and windows must provide equivalent manual and semantic actions.

## Data and authority

Manual mode without a model; hybrid and automatic modes under #24, independent of explicit resource/action grants and confirmation policy. [ADR-0002](architecture/ADR-0002-authority-and-delegation.md) replaces the earlier Low/Medium/Total planning labels with shared authority for all clients. No replacement tiers or mandatory templates are required. The owner delegates scopes and can revoke them. Discovery grants no authority. Identity is bound to sessions/handles, never trusted from model arguments.

Each provider connection declares its destination and transmitted data. Use synthetic test fixtures; keep secrets out of prompts/logs; select content before egress and record metadata without storing credentials. Web/file content grants no new privileged instructions. Screenshots are limited to the authorized selection. #5 records the policy decision; enforcement and credential handling are implementation work in #13/#23 and their service dependencies.

## Open decisions and issue ownership

#3 adopts architecture/boot. #4 pins tools and the versioned machine. #5 adopts minimum shared authority and mode/confirmation separation in ADR-0002. #6 specifies the initial [service contracts](SERVICE-CONTRACTS.md); their host schema checks do not satisfy M1 guest acceptance. #7 selects the engine/fonts and web gaps. #20 sets measured budgets. #33 defines timing. #39 pins MCP SDK/client/transport. #42 selects the executor channel. Decisions about components not yet implemented do not block the initial definition.

Changing target hardware, required scope or component location requires recording the reason and impact in #1 and updating this document. Evidence-based test-parameter adjustments must not be presented as a new universal capability.

Architecture review on 2026-09-10: #44 makes the existing requirement for user-mode file-service access to the kernel block driver an explicit prerequisite of #12. #5/#6 precede that interface; #20 records the service topology and capacity gaps before integrated H1 acceptance. This clarifies implementation order without changing mandatory v0.1 outcomes. The [systems roadmap](architecture/systems-roadmap.md) records the rationale and separately labels optional research experiments.

Authority refinement on 2026-09-10: the owner asked to reconsider both the old permission labels and the approach itself. H1 will prove scoped file work, helper restriction, revocation and manual progress with a deterministic client. A general delegation framework and template UI are not prerequisites. Full owner delegation and the original manual/agent/recovery outcomes are preserved; authority enforcement is not claimed from the decision document.

Implementation update on 2026-09-10: #44 supplies [bounded user-mode block access](BLOCK-ACCESS.md), including native SDK calls, owned DMA lifetime and separate-VM sector persistence. R02 still requires #12 file semantics, #13 supervision and #14 manual shell; raw-sector tests do not satisfy file persistence or integrated H1 acceptance. No mandatory requirement changes.
