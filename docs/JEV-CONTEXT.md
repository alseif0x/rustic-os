<!-- SPDX-License-Identifier: Apache-2.0 -->

# Optional JEV context-ranking experiment

This host-only pilot tests whether JEV can put relevant repository excerpts first
for a development task. It preserves every supplied excerpt and its provenance.
It does not yet remove context, choose coding models, change agent configuration,
or participate in CI gates. A person can use the same report without an agent.

## Run

Use Python 3.11+ from the repository root. No third-party package is required.
Prepare a request from explicit, tracked UTF-8 source ranges (inclusive,
one-based lines):

```sh
python3 -m tools.research.context_select prepare \
  --task 'Find the boundary between kernel mechanism and optional AI policy' \
  --source docs/architecture/ADR-0001-kernel-and-boot.md:38:43 \
  --source docs/LICENSING.md:5:15 \
  --source docs/requirements-v0.1.md:7:13 \
  --output artifacts/jev-context/request.json

python3 -m tools.research.context_select rank \
  --request artifacts/jev-context/request.json \
  --output artifacts/jev-context/baseline.json
```

The default ranker is a deterministic lexical baseline; it does not call JEV.
Inspect `request.json` before sending: it contains the task, source text and
questions that will leave the machine. Tracked files can still contain private
information. Preparation does not discover or upload other files.

For an explicitly requested live run, set `OPENROUTER_API_KEY` in the environment,
or put `OPENROUTER_API_KEY=your-key` in
`~/.config/rustic-os/openrouter.env` with owner-only permissions. The file is read
as data, never executed. Keep credentials outside the repository and reports.

```sh
python3 -m tools.research.context_select rank \
  --request artifacts/jev-context/request.json \
  --output artifacts/jev-context/jev.json \
  --live
```

`--key-file PATH` selects another credential file. Each live run makes at most one
request, with no automatic retries. An API failure saves a clearly marked lexical
fallback and returns exit code 2. Invalid local input is refused before sending.
Reports retain the full input, source ranges and hashes, request hash, ranking
and available provider usage/cost and elapsed time. They belong in ignored
`artifacts/`; they can contain the selected source text.

## Contract and limits

The adapter pins `typesafe/jev-1.13` and uses
`POST https://openrouter.ai/api/alpha/decisions`. This is OpenRouter's native typed
decisions endpoint, not chat completions. One shared state contains the task and
candidates; one `noul` question per candidate asks about relevance. Each question
names its candidate in its instructions because the question map key is not
model context. The response is a number from 0 to 1, used only for ordering.
It is not a correctness, authority or calibrated confidence guarantee.

The pilot accepts at most 32 candidates and 48,000 serialized UTF-8 request bytes.
The byte cap is an operational bound, not a tokenizer or a guarantee of fitting
the provider's context window. Oversize input is rejected, not truncated. The
transport has a 30-second timeout and bounded response reading; it refuses
redirects. Missing, extra, non-finite, mistyped or out-of-range answers cause a
fallback, as do transport and authentication failures. Original snippets remain
available even with a zero relevance score.

This small experiment cannot establish savings or general retrieval quality.
Before integrating any context filtering into the development workflow, use a
separate collection of representative tasks with human-labelled required
excerpts. Compare lexical and JEV ordering, required-excerpt recall within an
actual token budget, end-task correctness, latency and billed cost. Include
unrelated text, misleading instructions inside candidates and unavailable API
cases. Keep authority and review instructions outside any optional filtering.

## Possible OS follow-up

If host evidence justifies it, a later optional user-space service could rank
catalog actions or workspace search results. A native agent may use external API
inference; native execution does not imply local inference. The present pilot
does not implement guest networking, TLS, an agent or a native JEV service.
Manual commands and deterministic search must remain usable without it.

Kernel isolation, handles, permissions, validation and recovery remain
deterministic mechanisms under [ADR-0001](architecture/ADR-0001-kernel-and-boot.md).
The existing workspace acceptance in [#51](https://github.com/alseif0x/rustic-os/issues/51)
is independent of this experiment. No guest dependency, ABI or authority change
is introduced.

## Protocol provenance

Original stdlib implementation; no upstream SDK or community project code is
vendored. Protocol references checked on 2026-09-21:

- [OpenRouter model](https://openrouter.ai/typesafe/jev-1.13).
- [Official request schema](https://github.com/OpenRouterTeam/typescript-sdk/blob/1a09de8a9749c72450bade8a373ed2120a2865c0/src/models/decisionsrequest.ts).
- [Official response schema](https://github.com/OpenRouterTeam/typescript-sdk/blob/1a09de8a9749c72450bade8a373ed2120a2865c0/src/models/decisionsresponse.ts).
- [Official endpoint implementation](https://github.com/OpenRouterTeam/typescript-sdk/blob/1a09de8a9749c72450bade8a373ed2120a2865c0/src/funcs/alphaDecisionsCreate.ts).
- [TypeSafe primitives](https://docs.typesafe.ai/primitives) and
  [known model limitations](https://docs.typesafe.ai/model-jaggedness/jev-1.13).

The endpoint is alpha: recheck its contract before changing the pinned model or
adapter. Offline fixtures establish client behavior, not provider availability
or execution inside RusticOS.

## Initial evidence

A live connection check on 2026-09-21 sent one factual `noul` question with a short
synthetic state. OpenRouter returned `typesafe/jev-1.13-20260917` from `TypeSafe`,
a `0.98` answer, 297 input tokens, 20 reported output tokens, and USD 0.000012474
reported cost in 0.452 seconds measured at the client. The request used
`typesafe/jev-1.13`; reports must retain the resolved response model separately.
This single request establishes API access and observed wire shape only.
Local evidence: `artifacts/jev-context/connection.json` (ignored).

The three-excerpt command above then completed through the pilot CLI with the
same 4,982-byte request snapshot used for the lexical baseline. JEV ranked the
architecture excerpt first (`0.92`), requirements second (`0.81`) and licensing
last (`0.03`); the lexical baseline put requirements first and tied architecture
with licensing. Client latency was 504.735 ms; OpenRouter reported 1,584 input
tokens, 64 output tokens and USD 0.000066528. Both reports retained identical
original excerpt texts and request hashes. The expectations were selected by the
assistant for this illustrative example, not independently labelled evaluation
data. Evidence: `artifacts/jev-context/{request,baseline,jev,smoke-expectations}.json`.

Host validation: `python3 -m unittest discover -s tools/tests` passed 209 tests,
including nine pilot tests for explicit sources and bounds, offline preservation,
wire format, numeric answers, sanitization and failure fallback. A CLI run with
an intentionally missing credential file returned exit 2 and saved all baseline
rows (`artifacts/jev-context/unavailable.json`). `cargo xtask check` passed 418
Rust tests plus formatting, lints and builds. No new guest behavior is claimed.
