<!-- SPDX-License-Identifier: Apache-2.0 -->

# JEV integration plan for RusticOS and its development workflow

Date: 2026-09-21. Status: implementation sequence accepted by the owner; D1 has started.
Baseline: `main` at `deb24f9` ([PR #84](https://github.com/alseif0x/rustic-os/pull/84)).
This planning artifact does not approve a new ABI, adopt a new kernel architecture,
activate model hooks, add dependencies, or change the scope/status of existing issues.
Implemented subsets and evidence are recorded in the corresponding delivery guides.

Plan validation: local document links and named initial evidence-owner paths were
checked. Independent review found no material gaps in scope, prerequisites,
authority, fallback or acceptance. This is review of a proposal, not new runtime
test evidence. See [D1 delivery and evaluation](JEV-TRIAGE.md) for the implemented
subset and remaining acceptance.

## Outcome

Use typed semantic decisions where they reduce the effort needed to find evidence,
diagnose failures, notice relevant events and select existing actions. People,
conventional programs and optional agents must use the same service facts and
authority. An unavailable model must leave manual operation and deterministic
acceptance usable.

There are two delivery tracks: host development assistance now, and optional
user-space OS features after their functional prerequisites exist. Keep one main
foundation increment and at most one bounded host experiment active, following the
[systems roadmap](architecture/systems-roadmap.md). Do not turn every
work package below into concurrent implementation.

## Verified starting point and constraints

- The [existing pilot](JEV-CONTEXT.md) prepares explicitly selected source
  ranges and ranks them using a lexical baseline or one explicit OpenRouter request.
  All source text remains recoverable. It does not perform automatic context pruning.
- The live three-excerpt smoke check took 504.735 ms and reported USD 0.000066528.
  This establishes integration, not retrieval quality, calibration or savings.
  Validation of that increment passed 209 Python tests, 418 Rust tests and full CI.
- The current provider is OpenRouter, model identifier `typesafe/jev-1.13`, through
  `POST /api/alpha/decisions`. Preserve the resolved response model and provider.
  Native typed decisions use Noul, Choice and Score; they are not chat completions.
- The [guest SDK](SDK.md) has bounded explicit memory and IPC, but no
  networking, TLS, `std`, general allocator, floating-point or SIMD support.
  Python host tooling cannot simply be linked into the guest.
- Native task/file clients and recovery paths exist, but the complete general
  catalog and service-v1 mission remain open in #22/#43/#47. A specified event API
  is not proof of a generally available native event stream.
- #51 workspace capacity and #52 independent native application delivery remain
  foundation work. #36/#16/#17 own network driver, sockets/DNS and HTTPS;
  #38 owns inventory/adaptation, #23 the optional integrated agent, #39 MCP,
  and #37/#40/#41 desktop/browser expansion. Issue states were checked for this plan.

## Responsibilities and decision lifecycle

The proposed lifecycle is observation -> authorized bounded snapshot -> typed
questions -> validated answer or explicit uncertainty -> application policy ->
suggestion/event or existing authorized executor -> independent outcome readback.

| Component | Responsibility and boundary |
| --- | --- |
| Evidence collector | Read selected logs, source, service facts or catalog descriptors; record revision, hashes, ranges, time and incomplete coverage. Compute arithmetic and exact status checks locally. |
| Request/answer contract | Version the question set, candidate IDs and answer types. Distinguish `ok`, `uncertain`, `unavailable` and `invalid`; absent evidence is not a negative answer. |
| Provider adapter | Own credentials, serialization, bounded requests, timeout, response validation, usage and endpoint compatibility. Model/provider configuration stays outside the kernel. |
| Feature policy | Interpret the typed result for a specific feature. Thresholds are evaluated parameters, not permissions or correctness guarantees. |
| Executor/service | Recheck caller rights, resource versions, cancellation and operation identity; perform only existing permitted actions. Retain the established receipt/recovery rules. |
| Evidence and recall | Preserve original observations separately from decisions, expose what was not inspected, and allow recovery of hidden context. Source links are evidence pointers, not generated explanations. |

The first second consumer should reuse the pilot's transport and validation by
extracting only the genuinely shared host code. Keep evidence collection, ranking,
triage and policy separate. Do not build a general agent platform, daemon, Rust
crate hierarchy or SDK before a consumer needs that boundary. A future guest
implementation must use native Rust modules and guest contracts; it does not
depend on the host package or expose JSON/model types through kernel ABI.

Questions in a batch are independent. For action selection, supply complete
operation-target candidates, or one compatible target question per operation and
combine the selected answers locally. Do not ask a second question to depend on
an answer from the same parallel call. Include an abstain/unknown option where
the supplied evidence may not support a decision.

## Development work packages

| ID / order | Deliverable and concrete integration | Acceptance and fallback |
| --- | --- | --- |
| D0 — existing | Keep the current explicit context pilot as the reproducible baseline. | Preserve its offline mode and full original excerpts; no quality claim beyond the recorded smoke check. |
| D1 — next | A CLI for failure triage, inspired by Stanley: consume a completed build/boot/test report, bounded log regions and an explicitly selected diff. Deterministic parsing extracts exit status, panic/assertion markers, scenario identity and timeouts. Choice questions classify likely failure family, likely changed subsystem and missing evidence; Noul questions can assess relation to the supplied change or recurrence. Emit JSON plus a concise evidence-linked report. | Never change test outcomes, rerun jobs, edit code or call a shell from model text. Preserve unknown/conflicting/incomplete cases. With no provider, return the parser's facts and original logs. The D1 delivery includes the shared decision contract, offline fixtures and first labelled evaluation below. |
| D2 — after D1 | Semantic diff search and review preparation, inspired by jgrep/Jev Review: evaluate complete selected hunks, identify possible effects on persistence, cancellation, ownership or public contracts, and map code to candidate documentation/tests needing inspection. Use a reviewed path-to-document map plus semantic ranking. | Return candidates, locations, coverage and uncertainty, not an approval verdict. Whole mandatory review scope remains available. Rust initially uses unified diffs or explicit ranges; the inspected jgrep function extractor supports Python/Go only. Do not automatically rewrite documentation or claim an invariant is proven. |
| D3 — after measured retrieval value | Two-stage context selection: deterministic file/path/symbol lookup, then JEV ranking of bounded excerpts. Introduce opt-in hiding only after a separate retention test, with immutable original storage, visible omitted ranges and a recall command. | Keep task constraints, instructions, authority context and pinned critical evidence outside pruning. Keep uncertain/error-bearing material by default. If recall storage fails, retain full context. A CLI report is the first integration; client-specific hooks require actual hook compatibility evidence. Winnow's Claude hook is not a Codex integration. |
| D4 — after D1/D2 produce useful observations | Advisory worker monitoring, inspired by Foreman: on meaningful progress events or a bounded quiet interval, compare goal, diff, recent outcomes and existing deterministic counters. Report repeated attempts, possible scope drift or evidence still needed. | Initially compare silently against actual outcomes, then show advisories. Coalesce duplicate observations. No model-driven worker termination, completion, merge, permission grant, or automatic lowering/replacement of the owner's selected models/efforts. Such control would require a separate evaluated policy proposal. |

Expose D1/D2 through ordinary CLI/files first so a person and an agent can use the
same output. Optional CI integration should publish an advisory artifact after
the required deterministic job; inference failure cannot turn red CI green or
make passing CI depend on provider availability. Keep paid calls opt-in initially.
Never attach credentials to untrusted PR code; a later authorized post-processing
job may read inert exported artifacts with a separately bounded egress policy.

### D1 first increment: bounded implementation card

Objective: reduce time spent locating the cause and missing evidence of an actual
failed RusticOS check. Suggested placement: a cohesive sibling host package under
`tools/research/`, extracting shared provider code only when that consumer exists.
Keep the established context CLI compatible. These are placement proposals, not
new files or public commands already implemented.

Inputs: one identified run manifest, selected log excerpts, optional selected diff,
and a question-set version. Outputs: exact parser facts, candidate diagnosis,
supporting source references, missing evidence, confidence/uncertainty, inspected
coverage, request hash and measured usage. Separate exact observations from model
judgments in the report. A successful command means a report was produced, not
that a diagnosis is correct.

Start from existing evidence owners instead of adding a second test runner:
`tools/xtask/src/checks.rs` and `artifacts/check.log` for host checks;
`tools/boot_support/runner.py` and per-mode `result.json`/`serial.log` plus
`suite.json` for VM outcomes; `tools/sandbox_support/jobs.py` and
`artifacts/jobs/<id>/job.json`/failure exports for isolated execution. Preserve the
distinction between guest panic, expected-negative fixture success, host resource
limit and executor failure. Exclude raw volume images from model input; select
only necessary textual observations from the evidence manifests.

Implement in reviewable steps: (1) fixtures and deterministic evidence export,
(2) typed adapter contract and offline triage, (3) explicit live comparison,
(4) evaluation report and an adopt/adapt/defer decision. Root owns integration
and shared tests; use the existing Luna max worker and independent review profile
from [ORCHESTRATION.md](ORCHESTRATION.md). No change of roles is implied.

## OS work packages

| ID / order | Product behavior | Prerequisites and acceptance |
| --- | --- | --- |
| O0 — host prototype after D1 | Replay selected exported service/operation facts to test semantic attention signals and candidate-action ranking. Use the same decision-record concepts, but label every output as host fixture or actual exported guest evidence. | Reuse existing contract fixtures; where a native fact/event is absent, use an explicitly labelled simulation. This does not implement a guest service, resolve transport, or complete #22/#23/#39. Avoid adding a custom host/guest bridge merely for the experiment. |
| O1 — first native read-only consumer | An optional semantic-attention service, inspired by HA-Jev: classify groups of observed task/file/service events as needing attention, possible repeated failure, or related notices. Publish an inspectable advisory with source facts and freshness. | Needs actual scoped observation APIs and lifecycle behavior from #22/#47/#38 and native external inference prerequisites #36/#16/#17. Derive exact failures/timeouts/resource limits locally; JEV adds semantic grouping/priority. Test outages, revocation, stale data, bursts, service restart and manual dismissal. It emits suggestions, not process-control actions. |
| O2 — workspace retrieval | Rank permitted workspace search results and offer temporary semantic views such as documents relevant to a task or reports about a recurring failure. | Needs useful workspace listing/range access from the #51 service integration; the current guest v6 probe alone is insufficient. Candidate discovery and read/egress authorization precede inference. Preserve ordinary search and pagination, mark partial coverage, and recheck access when opening a result. Search ranking is not a stored file move or deletion. |
| O3 — action suggestions, then bounded execution | Use Choice over existing compatible catalog actions for a selected goal/resource, inspired by agent-desktop. Begin with visible suggestions. A later opt-in executor can perform already-authorized actions through ordinary services, then read back the result/receipt. | Requires the corresponding catalog contracts and authority checks in #22/#38; #23 is the integrated-agent consumer, not a prerequisite for a person to use suggestions. Carry resource version, scope and operation identity through execution; refuse stale/unknown candidates, recheck revocation, and reconcile lost replies without generating a new operation. Test manual and optional-agent parity on the same mission. |

The native service/executor runs inside RusticOS; its configured JEV inference
runs externally through OpenRouter. That separation must appear in evidence.
Future local inference or other external providers can implement a compatible
user-space decision adapter after their own evaluation; the plan assumes neither
local JEV weights nor that a general text model has the same typed semantics.
No inbound OS endpoint is required solely for inference. Protected credentials,
TLS trust/time, network egress scope and bounded cancellation belong to their
own user-service implementations under #16/#17/#23. Respect the guest SDK's
runtime limits: select a bounded response/probability representation during
native design, rather than assuming FPU support or importing host JSON/runtime code.

Normal independently delivered native applications depend on #52. A deliberately
embedded acceptance fixture can establish a narrower execution result earlier,
but must not be reported as independent delivery. GUI/browser integration follows
#37/#40/#41 by consuming these same services; the initial semantic features can be
used through a console without a desktop. MCP remains a separate #39 adapter.

For O1, coalesce event bursts, consult only on relevant state changes, and require
a local persistence/cooldown rule before repeating a notification. Record the
snapshot's age; unavailable/expired semantic state is distinct from false.
Disabling the service clears its subscription and pending work while preserving
manual controls and existing task facts. A new model answer cannot undo a user's
dismissal without a defined new-event rule.

## Shared operating policy to implement incrementally

- Start with explicit invocations; later add per-feature `off`, observe-only and
  advisory modes. Execution is an additional opt-in mode only for O3 after its
  authority and recovery acceptance. Each feature has its own disable path.
- Reuse current 32-candidate and 48,000-byte request caps until a measured consumer
  needs different bounds. A byte cap is not a tokenizer/context guarantee. Bound
  aggregate batches, in-flight calls, retries and wall time as well as each request.
  Interactive consumers need their own timeout behavior; never block a kernel path,
  critical service loop or manual command on an inference response.
- Configure a session/day call limit and a monetary stop policy before unattended
  calls. Reserve concurrent allowance before sending; include failed billable calls
  and retries when known. Unknown usage is explicit and must not authorize unlimited
  further calls. The first live evaluation proposes at most 200 calls and USD 1 of
  estimated budget, with a conservative reserve and stop on changed/unknown pricing;
  this plan does not activate spending or promise an exact provider billing cap.
- Limit outgoing data to the feature's authorized selection. Keep raw observations
  local and credentials outside reports, repository and guest images. Deterministic
  redaction may help, but is not proof of secret-free content; read permission and
  permission to send data externally remain distinct. Defer ambiguous egress scope.
- Cache only when needed: key by caller/scope, provider, resolved/configured model,
  question schema, complete state hash and candidate generation. Define TTL for
  time-dependent state; never reuse an answer to confer authority. Invalidate or
  re-evaluate on model/question changes, and bound memory/disk retention. Recall
  originals have independent retention: evicting a decision must not strand hidden
  context or make it appear inspected when it is not.
- Keep an evidence record: revision/run and observation identifiers, source hashes,
  coverage, question version, requested/resolved model, provider, answer/status,
  measured latency and billed usage when reported, cache status, local policy result
  and later observed outcome. Do not fabricate reasoning for a typed probability.

## Evaluation and promotion gates

These are proposed pilot gates, not measured achievements or universal calibration.
Freeze labels and thresholds before the held-out run, publish failures, and widen
the sample before making broad quality claims. A model change reopens these gates.

| Gate | Evidence required before promotion |
| --- | --- |
| Protocol / fallback | Offline tests for complete typed answers, unknown IDs, invalid ranges/numbers, transport failure and timeout, cancellation, limits, and all-original-data recovery. No API access in ordinary tests. |
| D1 usefulness | Start with 50 distinct labelled failure cases: 30 development/tuning, 20 held out by failure family/run so duplicate logs cannot leak labels. Include build/lint failures, expected-negative VM outcomes, host/environment timeouts, persistence/replay failures and genuinely insufficient evidence. Compare parser-only vs parser+JEV under the same evidence/time budget. Independent reviewer checks labels, selected diagnoses and supporting evidence. Report precision, unknown/abstention rate, coverage and top-three relevant locations; never convert expected-negative markers into an actual test failure without reading the harness result. |
| Advisory promotion | On the frozen D1 holdout, target at least 90% correct accepted classifications with at least 60% answered coverage, zero contradictions of deterministic pass/fail facts, and a measured improvement in locating relevant evidence versus parser-only. Count uncertain cases in coverage, never as correct predictions. Report sample counts and uncertainty; a small pass permits limited advisory use, not a general accuracy guarantee. |
| D2 review preparation | Evaluate separately on labelled hunks and affected-document pairs, including known persistence/authority changes. Measure candidate recall and false positives with a fixed review budget; retain the full diff. Do not inherit D1 scores or skip required review when no candidate is flagged. |
| D3 pruning | Before hiding anything, use a separately labelled retrieval set and compare lexical vs JEV recall within the same measured downstream token budget. Require every labelled critical excerpt to remain visible, all required-excerpt recall >=95%, and successful recall of every omitted block in the test set. Target >=20% fewer downstream input tokens with no measured end-task quality loss; include inference, cache, recall and prefix-cache effects in total cost. Failure keeps ranking-only mode. |
| D4 supervision | Compare advisories against replayed worker traces with independent labels for genuine progress/stuckness/drift. Record false interruptions and time-to-detection; no worker-control promotion follows merely from a probability threshold. |
| O1/O2 semantic quality | Label the actual notification/search workload independently. Compare rule-only or ordinary search with JEV under equal latency/resource budgets, including stale, revoked, missing and ambiguous observations. Author selects acceptable false-notification/miss tradeoff before live promotion; protocol correctness alone is insufficient. |
| O3 action quality and authority | On a fixed mission suite, compare manual/deterministic operation and JEV suggestions for goal success, wrong-action rate, abstention and steps. Independently require rejection of every forbidden, revoked, stale or malformed request and intact effect/receipt recovery in the guest. A semantic score never substitutes for these checks. |
| Operational value | Measure p50/p95 end-to-end time, actual usage, request/recall count, errors, memory and user correction effort per completed task. For initial asynchronous host triage, propose <=5 s p95 added latency on the small fixture set; report outages separately and keep deterministic output immediately available. Revisit consumer-specific budgets before native/interactive use. |

Use human-reviewed labels or an independent reviewer tied to executable evidence;
label provenance must be explicit. A model's assessment of another model is not
ground truth. Do not tune prompts on held-out cases or count near-identical runs
as independent successes. Savings are measured across the whole workflow, not
inferred from JEV's low advertised input price or a smaller context alone.

## Validation ownership and delivery order

Existing host commands: `python3 -m unittest discover -s tools/tests` and
`cargo xtask check`, as defined in [DEVELOPMENT.md](DEVELOPMENT.md).
Use applicable service contract tests from `tools/contracts/tests` when touching
those contracts. Native claims require the established boot/mission harnesses and
new positive/failure cases in the owning guest layer; a host replay cannot close
a native acceptance item. Coordinate shared build/VM runs under one owner and
never experiment on `artifacts/terminal/data.raw`.

Delivery sequence: D1 evidence+triage -> D2 diff/review preparation -> D3 measured
retrieval/recall -> D4 advisory supervision. O0 can replace, not add to, the active
bounded experiment when ready; O1/O2/O3 follow functional prerequisite evidence.
Preserve #51/#52 foundation priorities and the existing applied-research gates
instead of creating a second independently active architecture backlog.

Each implementation increment gets its own bounded task, evidence, review and PR;
merge only after required checks, then delete its transient branch. Keep failures
and explicit adopt/adapt/defer decisions. Do not estimate delivery dates for native
phases before the network, service and delivery prerequisites are implemented.
There are no new issue creations, issue closures, protocol adoptions or changed
user permissions in this plan.

## Community patterns and primary sources

The survey inspected source code at these revisions; it did not execute upstream
benchmarks or establish their production reliability. Original RusticOS work
should adapt patterns, not vendor entire projects. Any dependency or copied code
requires the existing [provenance process](LICENSING.md).

| Source | Pattern adopted for evaluation |
| --- | --- |
| [Stanley Code, formerly jev-code](https://github.com/devagrawal09/stanley-code/blob/85f39e71db0f615b8ce6161a6672a2bf955fa7fd/src/workflows/triage-failures.ts) | Bounded failure evidence, typed diagnosis candidates and explicit missing evidence. |
| [jgrep](https://github.com/keltokhy/jgrep/blob/fdceb6bdf79165a133667b8e57f3b7244545f0a2/src/jgrep/code_inputs.py) / [OpenRouter adapter](https://github.com/keltokhy/jgrep/blob/fdceb6bdf79165a133667b8e57f3b7244545f0a2/src/jgrep/core.py) | Complete diff units, source provenance and provider-aware caching. |
| [SemDecide](https://github.com/sharziki/semdecide/blob/33cf5c03c50e02e59df3f3ea81f0650f6b791545/src/reflex_guard/commands.py) | Composable typed CLI operations; distinguish uncertain results from provider errors. |
| [Jev Review](https://github.com/devagrawal09/jev-review/blob/31f89602797fb7bea007f8a480bf368bf564954e/src/review/workflow.ts) | Staged selection and bounded evidence inspection; do not inherit model-derived review verdicts. |
| [Winnow](https://github.com/GhalebDweikat/winnow/blob/51d80b945c74c8384bc47fa817179f668289afd8/sidecar/src/winnow/policy.py) / [recall](https://github.com/GhalebDweikat/winnow/blob/51d80b945c74c8384bc47fa817179f668289afd8/sidecar/src/winnow/mcp_server.py) | Conservative context retention and recoverable original text. |
| [Foreman](https://github.com/thruwire/foreman/blob/a7d21d18d306a0cb9f3e15acefbdb5663521405c/src/foreman/policy.py) | Progress/stuckness signals; exclude its model-threshold FINISH path from our acceptance authority. |
| [HA-Jev](https://github.com/AboveColin/HA-Jev/blob/6b884331ccf5371e6c2bfad9e9885c821c052159/custom_components/jev/coordinator.py) / [notification example](https://github.com/AboveColin/HA-Jev/blob/6b884331ccf5371e6c2bfad9e9885c821c052159/examples/01_laundry_reminder.yaml) | Event coalescing, semantic state, budgets and deterministic notification persistence. |
| [agent-desktop](https://github.com/lahfir/agent-desktop/blob/7a8e4a10281c7319733aa200fd79501f34529716/scripts/jev/policy.mjs) | Capability-filtered action candidates, compatible parallel questions and outcome readback. |

Provider references: [OpenRouter model](https://openrouter.ai/typesafe/jev-1.13),
[official typed response schema](https://github.com/OpenRouterTeam/typescript-sdk/blob/1a09de8a9749c72450bade8a373ed2120a2865c0/src/models/decisionsresponse.ts),
and the endpoint/request references in [the existing pilot guide](JEV-CONTEXT.md).
Recheck the alpha API when implementing a new answer type or changing the provider.
