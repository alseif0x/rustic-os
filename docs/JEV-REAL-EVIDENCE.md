<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native evidence import and historical evaluation

This guide records the D1 native-evidence increment evaluated on 2026-09-21.
It is a host-side, advisory workflow for already completed evidence. It does
not rerun a boot, sandbox or GitHub job, change a test result, or make a root
cause claim. The complete retained record is the
[historical evidence manifest](evidence/jev-triage-historical-v1.json), with
the selected source artifacts under
`tools/research/failure_triage_eval/historical/` and complete local preparations
under `artifacts/jev-real/live-v1/`.

## Import one native report

The importer reads one explicitly selected JSON report and writes a portable
manifest. The command is the same for each supported native source:

```sh
python3 -m tools.research.failure_triage import-report \
  --kind boot \
  --input artifacts/triage-input/boot-result.json \
  --run-id boot/example \
  --output artifacts/triage-input/boot-manifest.json

python3 -m tools.research.failure_triage import-report \
  --kind sandbox \
  --input artifacts/triage-input/sandbox-job.json \
  --run-id sandbox/example \
  --output artifacts/triage-input/sandbox-manifest.json

python3 -m tools.research.failure_triage import-report \
  --kind github-job \
  --input artifacts/triage-input/github-job.json \
  --run-id github/run/job \
  --output artifacts/triage-input/github-manifest.json
```

`boot` validates the reported outcome, return code, timeout fields, build and
image identity, and elapsed and timeout budgets. `sandbox` validates the
schema, job, revision and terminal status, while keeping any `guest_result`
inside the guest observation. `github-job` requires a completed job, a terminal
conclusion and a commit hash. A native `timed_out` conclusion is preserved as
timed out; a native `cancelled` conclusion stays cancelled.

Every manifest keeps the complete parsed report once under
`facts.native_report`. It also records
`facts.source_report = {kind, path, sha256}`, where the SHA-256 is over the
selected source file. Only validated scalar observations are promoted beside
those values. Extra report fields, command strings and paths are inert data;
the importer never executes them, and it refuses an input/output alias.
It does not invent a `harness_passed` fact from a matching expected outcome or
from log text.

## Select evidence explicitly

Preparation names the only log ranges that enter a request. Ranges are
inclusive and are copied, hashed and retained in the request:

```sh
python3 -m tools.research.failure_triage prepare \
  --manifest artifacts/triage-input/github-manifest.json \
  --log artifacts/triage-input/job.log:1227:1237 \
  --log artifacts/triage-input/job.log:1244:1247 \
  --output artifacts/triage-input/request.json

python3 -m tools.research.failure_triage diagnose \
  --request artifacts/triage-input/request.json \
  --output artifacts/triage-input/baseline.json
```

The historical evaluator reads only its catalog: each case names one report,
its SHA-256, and one or more log paths, line ranges and hashes. It rejects a
changed range, a changed report, or duplicate report bytes.
There is no GitHub discovery, log search or implicit context expansion. The
catalog and all prepared snapshots are frozen before a live request. A live
run makes one `diagnose --live` call per case, without retries; report fields
and model output are never executed and no CI job is rerun. Hash de-duplication
detects identical report bytes only; it does not establish independence when
two incidents have different serialization or different report content.

The freeze was created at `2026-09-21T00:52:15.894251+00:00` with catalog
SHA-256
`06da1d5cb84b5b6f5aded5408e301509116e415377925c5294b6d048a55ccf46`.
The frozen label policy is “observed failure family, not proven root cause.”

## Selected historical observations

The sample was deliberately selected from available CI artifacts: seven
failures and one successful control. It is biased and is not representative of
RusticOS failures or of a production workload. The expected column below is a
reviewed observed-family label, not an independently established cause.

| Run | Cohort / label | Selected observation | JEV result (confidence; accepted) | Native report SHA-256 |
| --- | --- | --- | --- | --- |
| `35205658000` | failure / environment | Workspace manifest was missing from the prepared image; no compiler diagnostic | environment (0.51; no) | `624d494be64fa78f553ecf0ea3b8ba0f1ee581af3f5f991585244a18a9a0f2bc` |
| `34664552375` | failure / assertion | Native contract validator rejected an incomplete recovery mission | assertion (0.99; yes) | `66f450e63d664420816bf499a921e0e407bbe17f53b0fa2c95838c44bb5ec72a` |
| `35539050008` | failure / assertion | Terminal acceptance expected a write but received `Unavailable` | assertion (0.97; yes) | `b4dd9a4b544f210b6377bbbb7859981dcb4e91858ef525bdb097056252ae8362` |
| `34698587182` | failure / executor | Sandbox controller reported `executor_error` | executor (0.86; yes) | `26fcbdcbc6f0aaab1c052d67c80d6c612b2f3ae5458deee8f84370a631b2c056` |
| `35221009174` | failure / assertion | `block-user` expected success but observed `boot_failed`; guest cause absent | executor (0.60; no) | `61364b3a7f7f3a1a5e0cf8d17beb58b9aebb6bc2538dabea34ac7f7260da87cf` |
| `35161711511` | failure / assertion | Measurement acceptance failed because its unchanged control regressed | assertion (0.83; yes) | `85339aaaf54714c327c10ecac229ea1d7ca512cf7e25efdb93c244ce960340fc` |
| `35539017737` | failure / timeout | Job conclusion was `cancelled`; a selected annotation says the 15-minute maximum was exceeded | timeout (0.98; yes) | `a18ee8e2e566a54ea82692eeccd526ea34689cc30edd615c7177b6d99063bd81` |
| `35547016442` | control / unknown | Completed success job verified deliberate panic/hang fixtures and all 22 scenarios | timeout (0.43; no) | `cf7c4cf440ff62eb61146b4444e8dc8f8b0538f32fec2cd263e82f168eb1be2f` |

The cancellation row illustrates why status and annotation stay separate. The
imported GitHub fact remains `cancelled`; the `timeout` label is based on the
explicitly selected check annotation, not on an inference from cancellation.
Likewise, the successful control supplies no inferred harness-pass fact. Its
panic and hang lines are deliberate negative fixtures, followed by verification
of the complete suite.

The `Unavailable` write row overlaps the v1 persistence wording, which allows
an explicit disk, storage, replay, durability, flush or write observation. Its
label records the observed assertion failure; it does not establish a
persistence root cause. The measurement row has the same boundary: in
`tools/measurement/series.py:98-112`, the baseline is compared with an
uninjected control and with an injected batch, and acceptance requires the
control to pass while the injected read regression is detected. The retained
job failed because the uninjected control itself regressed; the injected
regression was expected. This is an assertion-family label, not proof of a
code regression or timing cause.

## Result

The local signature baseline and JEV used the same frozen requests. For JEV,
`accepted` means the local 0.8 confidence gate admitted a non-unknown choice;
the reported confidence is not an established calibrated probability. The
baseline accepts a non-unknown signature without assigning confidence.

| Cohort | Cases | Baseline: correct / accepted | JEV: correct / accepted | JEV coverage | JEV abstentions |
| --- | ---: | ---: | ---: | ---: | ---: |
| Seven failures | 7 | 1 / 6 | 5 / 5 | 5/7 (71.4%) | 2 |
| Passed control | 1 | 0 / 1 | 0 / 0 | 0% | 1 |

The control was correctly left unaccepted only because its raw model proposal
was `timeout` at confidence **0.43**. That abstention does not demonstrate a
deterministic passed-harness guard. Across all eight calls the resolved model
was `typesafe/jev-1.13-20260917` from TypeSafe; usage was 41,017 input tokens
and 950 output tokens at a reported cost of **USD 0.001722714**. There were no
retries and no prompt or threshold tuning after observing these answers. The
committed [evidence record](evidence/jev-triage-historical-v1.json) includes
the summary, progress, freeze and all per-case live reports.

Promotion remains deferred. This small, biased sample does not establish
representative usefulness, retrieval benefit or developer time/token savings.
Keep the workflow in explicit research mode; the next acceptance should use
prospectively labelled failures with an explicit harness result and a separately
versioned evaluation, without tuning on this retained sample.
