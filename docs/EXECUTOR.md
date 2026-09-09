<!-- SPDX-License-Identifier: Apache-2.0 -->

# Isolated host executor (#21)

The owner can build a Git revision and test its kernel in QEMU without running candidate scripts directly in their session. No model, MCP or graphical interface is required. This is host infrastructure; it is not yet a RusticOS service or the #42 bridge.

## Usage

Baseline: Linux amd64, Python 3.12, Git and Docker Engine with cgroup v2. Validated on Ubuntu 24.04 under WSL2 and in CI. On Windows, run from Ubuntu; QEMU uses TCG without KVM or access to host devices.

From a reviewed infrastructure checkout:

```sh
python3 tools/sandbox.py prepare
python3 tools/sandbox.py run --revision "$(git rev-parse HEAD)"
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
python3 tools/sandbox.py cancel JOB_ID
```

`prepare` downloads tools and dependencies and builds the reference image. It is a trusted owner operation with network access that may take up to 15 minutes. Repeat it when reviewed tools or dependencies change. Do not run this step from unreviewed candidate code. Configuration is saved in `.cache/sandbox-image.json`: Ubuntu digest, immutable image identity and infrastructure file hash. A candidate revision's Dockerfile is not executed.

`run` requires the full SHA of a local commit. It exports that commit with `git archive`, excluding `.git` and uncommitted changes. It accepts no caller-supplied commands, mounts, environment variables, devices, URLs or Docker options. The only adjustments are `--mode ok|panic|hang|invalid|exception|gp|doublefault|timer-stall|memory-ro|memory-nx|memory-unmapped|memory-text-alias|memory-guard|block-persist|block-readonly|block-error|block-timeout|block-missing`, `--build-timeout 1..300` (default 120) and `--boot-timeout 1..120` (default 30). Additional modes cover deliberate boot, exception, timer-loss, memory-protection and block-device tests; see [INTERRUPTS.md](INTERRUPTS.md) and [MEMORY.md](MEMORY.md).

Standard output is a JSON object. Events go to stderr; the `started` event includes the identifier needed to cancel from another terminal. `Ctrl+C` and SIGTERM also request cancellation and cleanup. Exit code 0 means success, preparation or handled cancellation; 1 means failure. argparse syntax errors use 2.

## Boundaries and resources

1. One container builds the snapshot with offline Cargo and a disposable dependency registry. The target is `x86_64-unknown-none`; `RUSTIC_BUILD_ID` is set to the revision prefix.
2. Only a size-bounded ELF is exported through a read-only reference executable. The host receiver accepts a single regular member with the exact name; it rejects links, alternative paths and oversized data. It does not extract paths from the tar archive. Candidate bytes remain untrusted data, even if they change during export.
3. The build container is removed. A second container packages those bytes with reference Limine, configuration and notices, then runs QEMU. Candidate scripts do not control the boot evaluator. Success requires QEMU's exit status and the serial marker for the expected revision.
4. Artifacts are collected and both containers are removed, including their processes and tmpfs. Workspaces are not reused between jobs.

| Per-container limit | Policy |
| --- | --- |
| CPU / RAM / additional swap | 2 CPUs, 2 GiB, 0 |
| Processes | 128 |
| Workspace / temporary space | 1 GiB / 128 MiB tmpfs, included in RAM |
| Network / privileges | No external network, UID 1000, no capabilities, no-new-privileges, default seccomp |
| Filesystem | Read-only root; no host mounts, Docker socket or added devices |
| Retrieved snapshot / ELF / image | Maximum 32 MiB / 16 MiB / 64 MiB |
| Client logs / retrieved VM logs | 8 MiB per command / 1 MiB per log |
| Concurrency and attempts | One active job per checkout; one attempt, no automatic retries |

Time limits cover each stage: builds use the stated budget; packaging and VM execution allow 30 seconds beyond the QEMU timeout. Block persistence allows two per-VM budgets plus that margin. Creation, export and cleanup have their own deadlines of up to 30 seconds per command. Total elapsed time is not promised to equal exactly the sum of the two parameters.

Docker shares the host Linux kernel. These checks verify specific restrictions, not protection against vulnerabilities in Docker, the kernel or QEMU. For hostile third-party code requiring a stronger boundary, also run the Docker service and controller inside a disposable VM. A user authorized to manage Docker retains authority over the host: do not hand that socket or the user's shell to a guest or agent without mediation.

The probe uses a synthetic host file. It checks the absence of that file and the socket, denied root writes and external connections, UID, capabilities, seccomp, cgroups, tmpfs quotas and absence of added mounts/devices. It does not inspect personal documents. Any secret already included in an exported commit is part of the snapshot; the executor is not a secret scanner.

## Results and recovery

`artifacts/jobs/ID/job.json` records `schema_version: 1`, revision, tool image, controller/image-infrastructure/snapshot hashes, limits, observed container configuration, timings, guest result and artifacts with SHA-256 and size. `image.json` adds versions/hashes for Rust, QEMU, OVMF, Limine and the kernel/image. Logs and artifacts remain on the host after cleanup.

| Final status | Meaning |
| --- | --- |
| `success` | Build and boot of the expected revision verified |
| `build_failed` | Compilation returned an error |
| `build_timeout` | Build exceeded its budget |
| `boot_failed` | Packaging/VM failure or guest panic/fatal/unexpected exit; inspect logs and `guest_result` |
| `boot_timeout` | QEMU or boot-stage timeout |
| `resource_limit` | Docker reported an out-of-memory kill; other limits may appear as stage failures |
| `cancelled` | Cancellation handled and cleanup completed |
| `executor_error` | Controller or transfer error; no kernel failure established |
| `cleanup_failed` | Removal could not be confirmed; `cleanup_errors` needs owner attention |
| `request_error` | Invalid input/configuration or unavailable local tool |

`cancel ID` requests termination of the job's containers and checks their ownership label before removing them. Its `cancellation_requested` response confirms the handled request; read the full terminal status in `job.json`. After SIGKILL, reboot or Docker failure, run `cancel ID` again once Docker is available; if the controller is no longer active, it also reconciles abandoned state. Unrelated containers, owner images and artifacts are not deleted to hide errors. There is no host kernel installation to restore: rerun a known commit in a fresh workspace.

Job quotas do not include Docker provisioning or accumulated artifact history. The owner manages retention; CI keeps evidence for 14 days. There is no global Docker cleanup. The tool image is built locally and is not published to a registry; packages retain their own licenses.

## Verification and evidence limits

`test` runs twenty-two cases: success, panic, guest hang, invalid arguments, #UD, #GP, double fault, timer loss, five memory-protection faults, five block-device scenarios (including two-boot persistence), a real compilation error, a real build-script hang, cancellation during compilation and a successful clean repeat. Fixtures are unreferenced local commits created with a temporary Git index: they do not modify files, staging or branches. Hang/error cases require their marker to avoid accepting an earlier tooling failure. `artifacts/sandbox-suite.json` links the jobs; `artifacts/isolation-probe/result.json` records the verified restrictions.

CI provisions from its revision's code without persistent credentials or project secrets. An author who also changes the workflow or controller can change their own tests: a green PR is not external attestation against a malicious author. Testing separate candidates requires reviewed infrastructure and an owner-prepared image. Container isolation protects the host boundary; [#10](PROCESSES.md) separately checks ring 3 processes inside the guest. The `ok` case also requires those processes' protection, survivor progress and reclamation results, plus IPC exchange with buffer/handle validation. Preparation includes the original ABI crate without new Cargo downloads.

The executed image is pinned by content identity and Ubuntu by digest. Resolving all transitive packages during `prepare` is not hermetic: rebuilding on another date may produce another image identity, which must be retained in the evidence. The policy uses documented [Docker resource limits](https://docs.docker.com/engine/containers/resource_constraints/), [container execution](https://docs.docker.com/engine/containers/run/) and [seccomp](https://docs.docker.com/engine/security/seccomp/) mechanisms.

## Block storage acceptance

The five additional modes are `block-persist`, `block-readonly`, `block-error`, `block-timeout` and `block-missing`; they require successful negative/positive driver assertions and independent selected-sector verification. The trusted worker creates its own sparse 4 GiB disk; candidate input cannot select a host disk path. Each boot container permits a logical file size of 4 GiB, while the build container retains 256 MiB. Actual tmpfs/RAM quotas are unchanged. The disk never leaves the worker: only `block.json` (at most 64 KiB) and `blocks.bin` (2048 bytes) are exported, alongside ordinary bounded artifacts. See [BLOCK.md](BLOCK.md).
