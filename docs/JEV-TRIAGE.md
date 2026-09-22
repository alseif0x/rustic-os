<!-- SPDX-License-Identifier: Apache-2.0 -->

# Optional host failure diagnosis

This is the first D1 delivery of the [JEV integration plan](JEV-PLAN.md).
It produces an advisory from explicitly selected evidence. The original runner
facts and excerpts remain in the report. A category is a hypothesis, not a test
result or proof of root cause. Human development, checks and boot require no API.

## Use

Create a portable manifest from one identified run. `facts` are observations
supplied by the operator, not independently authenticated by this tool:

```json
{
  "schema_version": 1,
  "run_id": "ci-run-id/job/scenario",
  "runner": "boot",
  "facts": {
    "outcome": "panic",
    "returncode": 35,
    "timed_out": false,
    "expected_outcome": "panic",
    "harness_passed": true
  }
}
```

For boot evidence copy the actual fields from that run's `result.json`; only add
`harness_passed` from the harness result. Matching the expected outcome alone does
not prove the fixture reached its assertions. A deliberate panic or hang may be a
successful negative test. For sandbox jobs preserve `job.json` status and relevant
phase fields; for host commands record the actual exit status, not one inferred
from log text. Keep guest return codes distinct from the outer harness exit.
A command present in a manifest remains inert data.

Select only the log/diff ranges needed for the investigation:

```sh
python3 -m tools.research.failure_triage prepare \
  --manifest artifacts/triage-input/manifest.json \
  --log artifacts/triage-input/serial.log:1:20 \
  --output artifacts/triage-input/request.json
python3 -m tools.research.failure_triage diagnose \
  --request artifacts/triage-input/request.json \
  --output artifacts/triage-input/baseline.json
```

The default diagnosis uses local error signatures. A baseline `accepted` value
means a rule emitted a category; its confidence is null because no model
probability exists. Live acceptance has a separate confidence gate. Add `--live` to `diagnose` for one
OpenRouter native Decisions request, using `typesafe/jev-1.13`. Credentials follow
the [context pilot](JEV-CONTEXT.md): environment or the private
`~/.config/rustic-os/openrouter.env` file, optionally `--key-file`. Never copy a key
into evidence. Review selected content before sending it externally.

The snapshot records inclusive line ranges, hashes, original texts and question
version. Repeat `--log` or add `--diff PATH:START:END` for selected change context.
Selection is bounded to 32 excerpts, 4 MiB per source file and 48,000 serialized
request bytes.
These bounds do not guarantee a tokenizer's context limit. Original data remains
available on API failure; no model text is executed and no job is rerun.

A live Choice may omit confidence under the provider contract. That is a valid
answer but cannot pass the local 0.8 acceptance threshold. This threshold is an
experimental policy, not a calibrated probability of correctness. `unknown` and
passed-harness observations produce no accepted failure hypothesis. Missing or
invalid answers and transport failure remain explicit with a local fallback.
The report separates coverage and missing evidence from the diagnosis.

Exit 0 means a report was produced, including uncertain/offline reports; exit 1
means invalid local input; exit 2 means live inference unavailable with fallback.
Neither exit 0 nor a high score declares the underlying test successful.

## Evaluation

```sh
python3 -m tools.research.failure_triage_eval \
  --output artifacts/jev-triage/offline
# Explicit paid run; output directory must be new:
python3 -m tools.research.failure_triage_eval \
  --output artifacts/jev-triage/live --live
```

The initial dataset contains 50 original **synthetic** cases, split before live
execution into 30 development and 20 held-out scenarios. Scenarios use distinct
mechanisms, but share broad classes and synthetic distractors. Baseline contract
and signature fixes occurred during offline engineering checks, so this is not
a blind comparative benchmark. The split is retained for the first JEV run;
there was no tuning against live held-out answers. These are not 50
independent production failures. Labels stay outside provider requests. An
independent review checked label consistency before the freeze. This is fixture
review, not human confirmation of real root causes.

The evaluator preserves labels and their hash before requests, all snapshots,
local/live reports, pricing, progress and a summary. It reports accepted precision,
abstentions, coverage, a small-sample Wilson interval, and top-three evidence hits
on cases with an identified failure. Those intervals describe fixture counts;
they do not establish population accuracy. Four distractor excerpts avoid a
trivial top-three-of-three score. No downstream time or token savings are inferred.

A live batch allows at most 50 calls and USD 1 of estimated spending, reserving
USD 0.01 per call after checking the published input/output price. It stops if
pricing changes, a call's usage is unknown, or its reported cost exceeds the
reservation. This is a local estimate/stop policy, not a provider billing cap.
There are no retries. Ordinary tests and CI make no paid requests.

Promotion remains deferred until representative historical or prospectively
labelled failures show useful evidence selection against the local baseline.
Synthetic success alone does not satisfy the integration plan's usefulness gate.
The next D1 work includes native evidence exporters and richer change/missing-
evidence questions; this first adapter uses explicit portable manifests and
classification with evidence ranking. It implements no guest semantic service.

## First live result (2026-09-21)

The [retained measurement](evidence/jev-triage-v1.json) records 50 successful calls,
93,957 input tokens, 9,442 output tokens and USD **0.003946194** reported cost.
Resolved model: `typesafe/jev-1.13-20260917`, provider: `TypeSafe`.
Measured diagnosis-call latency was 331.580 ms p50 and 414.715 ms p95; this excludes
fixture preparation and downstream human investigation.

| Reserved 20 synthetic cases | Local signatures | JEV with local policy |
| --- | --- | --- |
| Correct / accepted hypotheses | 14 / 15 (93.3%) | 15 / 17 (88.2%) |
| Coverage of all 20 cases | 75% | 85% |
| Relevant excerpt among top three (16 eligible) | 15 / 16 | 16 / 16 |

JEV labelled a lint error as compilation (`case-197bce7a768b`) and inferred an
executor failure from a panic without sufficient evidence (`case-46c7755eca4c`).
On one development case, it proposed a timeout failure with confidence 0.99 even
though the harness passed its deliberate-hang fixture; local policy suppressed
that hypothesis and retained it separately for audit. All supplied facts survived
unchanged. The 95% Wilson interval for the 15/17 count is approximately 65.7–96.7%;
these synthetic counts do not establish real-world calibration.

**Decision: defer promotion.** The reserved-case accepted precision falls below
the proposed 90% gate, and the raw model did not respect every passed-harness fact.
The small evidence-ranking improvement is useful to investigate, but is not proof
of developer time savings. Keep explicit experimental invocations, collect actual
labelled failures and evaluate a separately versioned follow-up. The native import
and selected historical evaluation are recorded in [JEV-REAL-EVIDENCE.md](JEV-REAL-EVIDENCE.md)
with the [retained evidence](evidence/jev-triage-historical-v1.json). No prompt or
threshold was retuned after seeing these live answers.

Validation: `cargo xtask check` passed 418 Rust tests, formatting, Clippy and
builds; the separate Python suite passed 223 tests. This verifies host integration
and fallback contracts, not new guest behavior. A historical individual panic
artifact also yielded an offline abstention with the missing harness result
identified, rather than inventing a passing suite.

## Provenance

The workflow was informed by the inspected
[Stanley triage workflow](https://github.com/devagrawal09/stanley-code/blob/85f39e71db0f615b8ce6161a6672a2bf955fa7fd/src/workflows/triage-failures.ts).
This implementation and its synthetic examples are original; no upstream source
was copied and no dependency added. OpenRouter's
[Choice answer contract](https://github.com/OpenRouterTeam/typescript-sdk/blob/1a09de8a9749c72450bade8a373ed2120a2865c0/src/models/decisionschoiceanswer.ts)
allows optional confidence/probabilities. The kernel, guest SDK, services and ABI
are unchanged.
