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

CI selects `prepare --package-source github`, using `azure.archive.ubuntu.com` for both Ubuntu archive and security suites, as the hosted Ubuntu runner does. Local preparation defaults to the pinned base image's original sources. The selector is a closed set (`default` or `github`), is included in the infrastructure fingerprint and saved image configuration, and is unavailable to candidate `run` requests. It changes the download route; pinned package requirements, Ubuntu signature verification, firmware checksums, isolation restrictions and time budgets stay in force. This option addresses the slow default-mirror downloads observed in [publication CI attempt 1](https://github.com/alseif0x/rustic-os/actions/runs/34580495562/job/103202672070), whose 12m41s preparation exhausted most of the outer 20-minute job budget before the isolated suite could finish. That incomplete attempt remains historical evidence, not a passing run.

`run` requires the full SHA of a local commit. It exports that commit with `git archive`, excluding `.git` and uncommitted changes. It accepts no caller-supplied commands, mounts, environment variables, devices, URLs or Docker options. The only adjustments are `--mode ok|panic|hang|invalid|exception|gp|doublefault|timer-stall|memory-ro|memory-nx|memory-unmapped|memory-text-alias|memory-guard|block-persist|block-readonly|block-error|block-timeout|block-missing|block-user|block-user-faults|terminal-test|recovery-test`, `--build-timeout 1..300` (default 120) and `--boot-timeout 1..120` (default 30). Additional modes cover deliberate boot, exception, timer-loss, memory-protection and block-device tests; see [INTERRUPTS.md](INTERRUPTS.md) and [MEMORY.md](MEMORY.md).

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
| Recovery `result.json` / `recovery.json` | 128 KiB each; other summary files retain their 64 KiB limit |
| Concurrency and attempts | One active job per checkout; one attempt, no automatic retries |

Time limits cover each stage: builds use the stated budget; packaging and VM execution allow 30 seconds beyond the QEMU timeout. Block persistence allows two per-VM budgets plus that margin. Creation, export and cleanup have their own deadlines of up to 30 seconds per command. Total elapsed time is not promised to equal exactly the sum of the two parameters.

Docker shares the host Linux kernel. These checks verify specific restrictions, not protection against vulnerabilities in Docker, the kernel or QEMU. For hostile third-party code requiring a stronger boundary, also run the Docker service and controller inside a disposable VM. A user authorized to manage Docker retains authority over the host: do not hand that socket or the user's shell to a guest or agent without mediation.

The probe uses a synthetic host file. It checks the absence of that file and the socket, denied root writes and external connections, UID, capabilities, seccomp, cgroups, tmpfs quotas and absence of added mounts/devices. It does not inspect personal documents. Any secret already included in an exported commit is part of the snapshot; the executor is not a secret scanner.

## Results and recovery

`artifacts/jobs/ID/job.json` records `schema_version: 1`, revision, tool image, controller/image-infrastructure/snapshot hashes, limits, observed container configuration, timings, guest result and artifacts with SHA-256 and size. `image.json` adds versions/hashes for Rust, QEMU, OVMF, Limine and the kernel/image. Logs and artifacts remain on the host after cleanup.

When the trusted boot worker exits with an error, the controller records its exit code and attempts one bounded evidence export before removing the container. It retains available `image.json`, `result.json`, combined serial/QEMU logs and a `boot-failure-capture.json` report. Recovery failures additionally retain an available `recovery.json`, select a whitelisted session named by valid `failure-NAME.json` metadata and retain its raw 89,088-byte format prefix, final 512-byte sector, serial/QEMU logs and command timings. Timings contain command verbs and elapsed observations, without arguments; a failure before command input can leave them empty or absent. The uncompressed bundle is limited to 8 MiB, each timing file to 2 MiB, and each other artifact to its explicit bound. The receiver rejects unknown paths, links, duplicate members and oversized files, then writes only validated fixed filenames. No archive paths are extracted and no directory is copied recursively.

`failure_evidence` in `job.json` records missing/rejected files and capture errors independently of the original `boot_failed` or `resource_limit` status. Missing evidence never turns a failed execution into success or an evidence-transfer error into a different kernel result; cleanup failures still take precedence as `cleanup_failed`. This capture applies after a worker exits with a nonzero status. Controller cancellation, a worker that exceeds its overall time budget or an unavailable container may leave only existing controller logs. Repeat `prepare` after changing the reviewed exporter so the pinned image contains it. Captured disk bytes are observations after the owned VM stops, not proof of durability at the earlier failure instant.

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

`test` runs twenty-six cases: success, panic, guest hang, invalid arguments, #UD, #GP, double fault, timer loss, five memory-protection faults, seven block scenarios (including kernel and user-mode two-boot persistence), native terminal UART/file-service acceptance across two boots, a real compilation error, a real build-script hang, cancellation during compilation and a successful clean repeat. Fixtures are unreferenced local commits created with a temporary Git index: they do not modify files, staging or branches. Hang/error cases require their marker to avoid accepting an earlier tooling failure. `artifacts/sandbox-suite.json` links the jobs; `artifacts/isolation-probe/result.json` records the verified restrictions.

CI provisions from its revision's code without persistent credentials or project secrets. An author who also changes the workflow or controller can change their own tests: a green PR is not external attestation against a malicious author. Testing separate candidates requires reviewed infrastructure and an owner-prepared image. Container isolation protects the host boundary; [#10](PROCESSES.md) separately checks ring 3 processes inside the guest. The `ok` case also requires those processes' protection, survivor progress and reclamation results, plus IPC exchange with buffer/handle validation. Preparation includes the original ABI crate without new Cargo downloads.

The executed image is pinned by content identity and Ubuntu by digest. Resolving all transitive packages during `prepare` is not hermetic: rebuilding on another date may produce another image identity, which must be retained in the evidence. The policy uses documented [Docker resource limits](https://docs.docker.com/engine/containers/resource_constraints/), [container execution](https://docs.docker.com/engine/containers/run/) and [seccomp](https://docs.docker.com/engine/security/seccomp/) mechanisms.

## Block storage acceptance

The five additional modes are `block-persist`, `block-readonly`, `block-error`, `block-timeout` and `block-missing`; they require successful negative/positive driver assertions and independent selected-sector verification. The trusted worker creates its own sparse 4 GiB disk; candidate input cannot select a host disk path. Each boot container permits a logical file size of 4 GiB, while the build container retains 256 MiB. Actual tmpfs/RAM quotas are unchanged. The disk never leaves the worker: only `block.json` (at most 64 KiB) and `blocks.bin` (2048 bytes) are exported, alongside ordinary bounded artifacts. See [BLOCK.md](BLOCK.md).

#44 adds `block-user` (two independent boots) and `block-user-faults` to the same disk-owning worker. Both require complete ring 3 evidence, control progress and recovery as well as the host oracle. The build worker exports `sdk-probe.elf`, `app.manifest`, `block-probe.elf` and `block-probe.manifest` with independent hashes; each ELF is limited to 1 MiB and each manifest to 128 bytes. No device, host path or command can be selected by a guest. [Native block contract and acceptance](BLOCK-ACCESS.md).

## Native terminal acceptance

The terminal-test mode uses the reviewed terminal_support driver against the exact candidate ELF. The build worker exports all six application ELFs and manifests with bounded sizes/hashes. The boot worker owns its data disk and drives real UART input; the candidate cannot supply a host script, path or command. After two boots, an independent reader verifies persisted files and untouched reserved sectors. Only terminal.json (64 KiB limit) and files.bin (89,088 bytes) leave the worker alongside the normal logs/image. The original sparse disk is removed. The boot-stage wall limit is four times the selected per-interaction timeout plus 30 seconds; quotas and container ownership rules are unchanged.

The recovery-test mode runs the same reviewed driver against the exact candidate ELF across 44 groups and 88 boots. It exports recovery.json and result.json (128 KiB each) and files.bin (89,088 bytes). Normal collection and failure capture share those fixed summary budgets. The expanded recovery evidence exceeds the former 64 KiB ceiling; this host export allowance does not change guest resources or timeouts. Tests cover complete collection at 128 KiB, rejection at 128 KiB plus one byte, cleanup after rejection, and unchanged 64 KiB result limits for other modes. Its boot-stage wall limit is sixteen times the per-interaction timeout plus 30 seconds, covering the bounded sequence of fresh VMs; no VM, disk or path is selected by candidate input. The existing CPU, memory, disk, network and container ownership limits remain in force. See [legacy recovery](FILE-RECOVERY.md), [workspace operations](FILE-OPERATIONS.md), [owner-control races](FILE-CONTROL.md) and [scheduled failure acceptance](FILE-SCHEDULING-FAILURES.md). The reviewed infrastructure includes both read and replacement fixture corpora; rebuild it with prepare after changing those drivers or fixtures.

The two-VM `block-user` fixture additionally exports `admission-terminal.bin` and `admission-pending.bin`, each bounded to 89,600 bytes including its trailing guard sector, alongside `publication.bin`. The independent reader validates [format-4 admission states](FILE-ADMISSION.md) and unchanged replay hashes. The isolated suite assigns 45 seconds per VM to its four sequential application fixtures, matching direct CI; other timeout selections and all guest resource/event limits remain unchanged.
