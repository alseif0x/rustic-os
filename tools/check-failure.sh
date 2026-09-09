#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Run only in a disposable checkout: the fixture is removed on exit.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture=kernel/tests/ci_failure_probe.rs
if [[ -e "$fixture" ]]; then
  echo "Refusing to overwrite $fixture" >&2
  exit 1
fi
trap 'rm -f -- "$fixture"' EXIT
cat > "$fixture" <<'RUST'
// SPDX-License-Identifier: Apache-2.0
#[test]
fn deliberately_fails() {
    panic!("RUSTIC_CI_FAILURE_PROBE");
}
RUST
# Format first so the probe reaches test execution, not the format gate.
cargo fmt --all
mkdir -p artifacts
if cargo xtask check > artifacts/failure-probe.log 2>&1; then
  echo "ERROR: check accepted a deliberately failing test" >&2
  exit 1
fi
grep -q RUSTIC_CI_FAILURE_PROBE artifacts/failure-probe.log
grep -q 'test result: FAILED' artifacts/failure-probe.log
echo "Failure propagation verified."
