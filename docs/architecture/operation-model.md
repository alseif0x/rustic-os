<!-- SPDX-License-Identifier: Apache-2.0 -->

# Operation boundaries: a finite design experiment

Date: 2026-09-10. Model version: 1. Status: executed host experiment supporting the [systems roadmap](systems-roadmap.md). Original Python standard-library code; no external dependency, model or network. It does not define product schemas or implement #5, #6, #12, #13 or #43.

## Question and reproduction

Can admission-time permission checking, resource-version checking alone, or a separately persisted receipt support reliable handoff and retry? Compare deliberately weaker designs with combined validation and an assumed local transaction.

From the repository root, using the existing Ubuntu reference host (Python 3.12.3 in the local run):

```sh
python3 -m tools.research.operation_model > artifacts/operation-model.json
```

The output is deterministic JSON; create `artifacts/` first in a fresh checkout. Exit status 0 requires enumeration of all 60 schedules, detection of deliberate faults in both weaker strategies, no observed violation in the combined strategy, some successful work rather than blanket denial, and detection of the split-receipt crash gap. CI runs the same command and preserves the result with workspace artifacts.

The source is split by responsibility: [concurrency](../../tools/research/operation_model/concurrency.py) enumerates/interprets event orders, [recovery](../../tools/research/operation_model/recovery.py) models crash cuts and [entry](../../tools/research/operation_model/__main__.py) reports results and checks the expected distinctions. There is no Rust or guest implementation change.

## Concurrency model

One writer has `observe < authorize < commit`. An owner performs one independent `human_edit`, and `revoke < regrant`. Enumerate all permutations satisfying these order constraints: 6! / (3! × 2!) = 60. The file version and delegation generation start at zero. A human edit increments the file version; revoke and regrant each advance the delegation generation. No expiry, counter wrap, resource rebinding, hierarchy or multiple grants is modeled.

Admission remembers whether authority was allowed and its generation. The three strategies differ only at commit:

| Strategy | Commit rule | Commits / 60 | Orders with an invariant violation |
| --- | --- | --- | --- |
| Admission only | Rely on the earlier admission | 36 | 23 |
| Version only | Earlier admission and unchanged observed file version | 18 | 5 |
| Version and authority | Earlier admission, unchanged file version, allowed current grant and matching generation | 13 | 0 |

The observer independently checks each accepted commit's pre-state: it must not overwrite an intervening edit or act with obsolete delegation. Both properties are checked regardless of which strategy executes. Regrant does not make an old generation valid again. These are constructed traces, not a statistical sample or failure probability estimate.

Example detected edit race: observe → authorize → human edit → commit. Example remaining in the version-only design: observe → authorize → revoke → commit → human edit → regrant. The combined design rejects both. A valid trace still commits, so rejecting every operation cannot satisfy the experiment.

**Critical assumption:** combined validation and the local effect are one atomic transition, serialized with edit and revocation. Real service/kernel code must define and enforce that boundary. If a worker validates and later writes without fencing, the model's result does not apply. Multi-service revocation needs an acknowledgment/fencing protocol before promising takeover completion; no such protocol is implemented here. Effects already committed are not undone.

## Recovery model

For one local effect with the same retry key and arguments inside retention, insert a crash at each of five cut points around prepare, effect, receipt and reply. Python variables stand for hypothetical durable state; this is not real persistence or a filesystem test. A lost reply triggers a retry, suppressed only if its durable receipt exists.

| Assumed durability | Cut points | Incorrect outcomes |
| --- | --- | --- |
| Effect and receipt saved separately | 5 | 1 duplicate: crash after effect, before receipt |
| Effect and receipt saved atomically | 5 | 0 |

An alternative implementation may use a durable intent plus idempotent recovery instead of an atomic effect/receipt pair. This experiment does not compare that design. It also does not model external network effects, argument conflicts, denial of receipt reads, retention expiry, reboot epochs or real disk failure. Those remain service-specific contract and acceptance cases.

## Decision and evidence limits

Seven runner checks pass for the local experiment. Results cover 60 schedules per strategy and five cuts per durability variant. CI records the source revision; local output is in ignored `artifacts/operation-model.json`. The evidence does not increase the counts of Rust, Python unit, VM or isolated executor acceptance tests in the README.

Carry these requirements into #5/#6 and later #13/#15/#43: defined commit/revocation ordering, resource preconditions, authenticated receipt lookup, explicit retention and recovery behavior. Unknown external outcomes need reconciliation; do not infer exactly-once external execution from the local model.

The experiment is finite exploration of a deliberately small model. It is not formal verification of RusticOS, a performance benchmark, proof of novelty, or evidence of filesystem crash consistency. The combined strategy encodes atomicity as an assumption; the difficult implementation work still lies ahead. Review was by the implementing agent without independent audit.
