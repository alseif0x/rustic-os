<!-- SPDX-License-Identifier: Apache-2.0 -->

# Explicit boot harness evidence for triage

This D1 increment connects the boot suite's existing acceptance decision to
host triage. It changes evidence export, not the guest or the test's acceptance
criteria. The classifier, questions and 0.8 confidence threshold remain v1;
the earlier synthetic and historical evaluations remain frozen.

## What the harness records

`python3 tools/boot.py test --timeout 45` writes a per-mode `harness.json`
after a run returns and its fixture-reached predicate has been evaluated.
The report identifies `rustic-boot-suite/v1`, a fresh suite run ID and the mode,
records expected and observed outcomes, and records both `reached_fixture`
and `harness_passed`. Pass requires the existing outcome comparison **and**
the existing reached predicate. A timeout before the deliberate-hang fixture
is reached therefore records failure, even though timeout is the expected outcome.

The report binds the raw `result.json` and `serial.log` with SHA-256 hashes
and carries the build and image identities. A returned result must agree with
the persisted result before an acceptance report is published. Prior per-mode
harness reports are cleared at suite start, including modes not reached in a
failed run. A completed mode's failed acceptance is exported before the suite
raises. A build/runner exception before a result returns has no acceptance
report; absence is unknown, never pass. `suite.json` still exists only after
all scenarios pass. An individual `boot.py run` does not issue suite acceptance.

These are local evidence records, not signed attestations. They prevent accidental
association with different result bytes; a hash does not authenticate an artifact
producer or establish independence between related runs.

## Explicit import

From a completed suite or a preserved CI artifact, select both files explicitly:

```sh
python3 -m tools.research.failure_triage import-report \
  --kind boot \
  --input artifacts/boot/hang/result.json \
  --harness artifacts/boot/hang/harness.json \
  --run-id my-preserved-suite/hang \
  --output artifacts/triage-input/hang-manifest.json

python3 -m tools.research.failure_triage prepare \
  --manifest artifacts/triage-input/hang-manifest.json \
  --log artifacts/boot/hang/serial.log:1:3 \
  --output artifacts/triage-input/hang-request.json

python3 -m tools.research.failure_triage diagnose \
  --request artifacts/triage-input/hang-request.json \
  --output artifacts/triage-input/hang-diagnosis.json
```

Choose actual line ranges for the evidence being investigated. Import validates
schema, typed acceptance fields, source-result hash and result identities before
promoting `harness_passed` and `expected_outcome`. It preserves the complete
harness report and its source hash. It never follows report paths, runs commands,
reconstructs pass from log text or derives per-mode acceptance from an entire
GitHub job. The optional attachment is supported only for `--kind boot`.
Without it, the existing importer behavior remains unchanged.

The boot importer retains its existing bounded native format: terminal and
recovery aggregate reports that omit `timeout_seconds` are still unsupported,
as are reports exceeding its byte bound. Do not fill missing fields with guessed
values. The attachment binds the result; arbitrary additional excerpt selections
remain the caller's responsibility. The offline native-control check also verifies
the selected complete serial file against its recorded hash.

The existing deterministic policy suppresses accepted failure hypotheses when
`harness_passed` is true, including a high-confidence model proposal; live reports
retain the raw model hypothesis separately. Explicit false acceptance prevents
the expected-negative shortcut from hiding a failed fixture. This behavior is
covered with host tests; it does not establish semantic diagnostic accuracy.

## CI and prospective collection

After the real boot suite, CI runs:

```sh
python3 -m tools.research.failure_triage.check_harness \
  --boot-directory artifacts/boot \
  --output artifacts/boot/triage-harness
```

This reads the actual panic and hang controls, checks their serial hashes,
imports their bound acceptance, prepares explicit excerpts and diagnoses them
offline. Both must retain their observed outcomes and the `harness_passed` guard.
The output directory must be new. There are no provider calls or credentials;
requests, diagnoses and a summary are retained with the boot artifacts.

For a new unexpected failure, preserve the original result, harness report when
present, relevant text logs and revision before another run overwrites them.
Assign an incident group to avoid counting retries or related jobs as independent
failures. Record the label, rationale, source hashes and uncertainty separately
**before** any model call; a failed assertion is not proof of its underlying cause.
Have the label independently reviewed, then prepare a bounded request without
including the label. Preserve passing deliberate-negative controls separately.
A missing harness result must remain missing. Do not upload raw volumes or secrets.

This increment supplies the collection mechanism and native controls. It does
not manufacture new unexpected failures, enlarge the prior historical score,
or satisfy the representative D1 usefulness gate. A future evaluation needs
prospectively labelled failures and a separately frozen dataset; routine JEV
promotion remains deferred. See [the integration plan](JEV-PLAN.md) and
[historical limitations](JEV-REAL-EVIDENCE.md).
