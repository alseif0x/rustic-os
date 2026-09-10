<!-- SPDX-License-Identifier: Apache-2.0 -->

# Repeated R0 measurements

The #20 harness measures the existing native kernel and owner-control workload. It builds once, freezes two images containing the same ELF, and starts fresh QEMU processes, firmware-variable copies and disposable data disks for every repetition. Measurement collection, guest workload, provenance and statistical decisions live in separate host modules under `tools/measurement/`. No measurement policy or new dependency enters the kernel.

## Run and compare

Use the [reference Ubuntu environment](DEVELOPMENT.md), including WSL2:

```sh
source ~/.cargo/env
python3 tools/measure.py verify --host-label local-wsl-r0 --samples 5 \
  --output artifacts/measurements/r0-check
```

The output directory must be new. This command collects three batches: baseline, a separate unchanged control, and a batch with a real injected service delay. Each batch has one excluded warmup and five measured repetitions. Each repetition boots the kernel diagnostic and the native terminal, so successful verification uses 36 actual VM boots. Between five and thirty measured repetitions are supported. The user terminal disk is never selected.

For later revision comparisons, collect each revision on the same host configuration and compare its reports:

```sh
python3 tools/measure.py run --host-label local-wsl-r0 --samples 5 \
  --output artifacts/measurements/candidate
python3 tools/measure.py compare artifacts/measurements/r0-check/baseline/report.json \
  artifacts/measurements/candidate/run/report.json \
  --output artifacts/measurements/comparison.json
```

Comparison exits 0 for pass, 2 for a measured regression, and 3 when reports are incompatible or incomplete. CLI/collection failures exit 1. Failed samples and warmups invalidate the collection; they are retained with partial metrics, status and logs, never silently removed to improve a distribution. A changed host or workload needs a separate baseline. Keep the host quiet and do not run a competing build/VM workload during collection; the lock prevents two measurement collectors in this checkout, not unrelated host work.

## What executes

The kernel `ok` fixture boots the candidate ELF and validates real frame exhaustion/recovery, page protections, process isolation, IPC, SDK calls and complete reclamation. Its serial records supply allocator metadata, kernel page-table frames, process-manager metadata and the fixture's simultaneous process-frame peak. Its whole diagnostic duration is not advertised as kernel boot latency.

The second VM initializes a fresh bounded native store, reaches the owner prompt and performs this fixed workload:

1. Create A/B, read A and explicitly restart files.
2. Provision C and read-only H, assert denial of B and privileged control, stage a write, then fill both actor queues. Reject a third utility, measure owner control/write/revocation, check queued denial and reclaim both actors.
3. Fill all remaining file-object slots, require `Full`, measure owner control and remove the temporary files.
4. Stop the service indefinitely, measure independent owner control and interrupt a foreground read with Ctrl-C. Restart and verify baseline counters.
5. Hold kernel completion observation after an actual first-data-write submission for 200 PIT ticks. Leave the write wait with `Uncertain`, measure owner control while `pending_io=1`, inspect the restart drain phase and collect the new binding.
6. Verify A and untouched B, shut down, require equal before/after free frames, and independently decode the disk checksums, contents and exact final file set (owner policy and A/B), proving temporary object reclamation.

This reuses the real mechanisms in [authority](AUTHORITY.md) and [foreground recovery](FOREGROUND-CONTROL.md). The delayed observation in step 5 is an injected workload, not measured device latency. QEMU may already have completed the physical write. Every successful sample keeps the native transcripts and the independently inspected disk prefix.

## Metrics and boundaries

| Metric | Interpretation |
| --- | --- |
| `boot_ready_seconds` | Host time from entering the native VM setup to the first usable prompt; includes image verification, firmware-copy/process/socket setup and UEFI/loader/service startup; excludes compilation and data-image provisioning |
| `read_seconds` | One verified foreground read round trip |
| `pressure_write_seconds`, `pressure_control_seconds`, `revoke_seconds` | Owner write, memory query and acknowledged session fence while both actor queues are full |
| `full_control_seconds` | Memory query with all file-object slots occupied |
| `stopped_control_seconds`, `interrupt_seconds` | Independent memory query and Ctrl-C-to-prompt while files does not read its channels |
| `admitted_control_seconds`, `drain_seconds` | Memory query with real submitted I/O pending, and collection of the explicit restart during the 200-tick observation hold |
| `kernel_load_bytes` | Union of page-rounded ELF `PT_LOAD` virtual ranges, including BSS and embedded catalog/test programs; a static reservation, not total physical resident memory |
| `kernel_page_table_frames`, `allocator_metadata_bytes`, `manager_metadata_bytes` | Existing native diagnostic accounting; overlapping/static categories must not be blindly added together |
| `process_fixture_peak_frames` | Native fixture's measured simultaneous process frames, including its stacks/page tables; not a terminal-wide maximum |
| `resident_runtime_frames` | Within the same terminal VM, pre-runtime free frames minus the three-process resident checkpoint; includes native process mappings/page tables and device allocations |
| `sampled_pressure_extra_frames` | Within that VM, resident free frames minus the C/H queue-pressure checkpoint; sampled growth, not a continuous peak or stack high-water mark |

All times use the host monotonic performance clock and include UART transport, scheduling and transcript writes. Guest PIT ticks are separate. The initial warmup is excluded, but host filesystem caches remain warm; this is not a physical cold-boot experiment. The payload is small and the store is bounded. Do not infer throughput, real-hardware latency, energy, battery life or production security from these results.

## Baseline and regression decision

Every metric retains each measured sample plus count, minimum, median, maximum and median absolute deviation (MAD). Five observations do not justify a tail-latency percentile or confidence guarantee; the report deliberately supplies no p95/p99 claim.

The first budget is derived from the observed baseline:

- Timing upper limit: baseline median plus the largest of 50% of that median, six unscaled MADs, and an absolute noise floor.
- Noise floors: 50 ms for command round trips; 250 ms for boot and the deliberately held drain. These are conservative engineering allowances for the reference timer, polling, logging and host variation, not confidence intervals or product SLOs.
- Memory counters: baseline maximum plus the explicit allowance in `measurement/model.py`. Frame/load-page metrics allow one page/frame; fixed metadata counters allow no growth. Candidate maxima are checked, so growth in one repetition cannot hide in a median.

An unchanged held-out batch must pass before this calibration can succeed. The injected batch issues `stall files 100` immediately before the measured read. The real file service waits for 100 PIT ticks; neither the host stopwatch nor its samples are adjusted. Both control and injected workloads restart afterwards. Verification requires the measured read to cross its previously established upper limit. An injected run cannot establish a baseline, and a timeout is incomplete evidence rather than successful regression detection.

Passing means no regression was detected by this bounded experiment. It does not establish a false-positive rate, a universal budget or absence of smaller regressions. Intentional resource growth requires reviewing its cause and rerunning calibration; a tool must not silently widen a baseline to accept a candidate.

## Provenance and evidence

Each report binds the guest source revision/worktree status, guest-source build ID, kernel/image hashes, reference toolchain/firmware, guest CPU/memory/disk/network parameters, and a hash of the measurement and transport/fixture code. Dirty source status is explicit: a commit alone does not identify that build. Frozen kernel/image hashes identify the binaries actually run.

Configuration matching includes host label, OS/kernel, CPU model/count/affinity, memory, Python/Rust versions and output/temporary filesystem types. The Linux collector resolves its cgroup v2 membership and records CPU/memory maximums and effective CPU/NUMA sets at every ancestor, so a stricter parent cannot disappear behind an unlimited leaf. Unsupported legacy/threaded layouts, unreadable controls and delegated namespace views that hide the root are rejected. The supported unified domain layout follows the [kernel cgroup v2 documentation](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html); this is configuration evidence, not an attestation of all host constraints. A hashed host boot identity keeps different host boots out of one baseline; no hostname, username or raw boot identifier is stored. The collector rechecks configuration before and after each sample; a change invalidates that sample. It records the actual temporary directory filesystem selected by Python, including TMPDIR overrides. Host load averages before/after samples are observations, not a filter that discards inconvenient data. Frequency scaling, shared runners, background work and host device/cache behavior remain sources of variation.

Local, container and CI reports are separate configurations. These experiments use direct reference QEMU execution; the existing 26-case isolated executor suite remains a separate correctness/security boundary. A future isolated measurement backend needs its own explicit configuration and baseline.

`report.json` preserves all attempts and distributions. Per-sample `sample.json` records hashes for serial/QEMU logs, native diagnostic results and the checked disk prefix. `control-comparison.json`, `regression-comparison.json` and `verification.json` preserve limits, outcomes and case counts. The collector refuses existing output directories. These are evidence records, not signed or tamper-proof audit receipts.

The `measurements` GitHub Actions job runs a fresh calibration, unchanged control and native regression on every push/PR and preserves evidence for 14 days. It is an infrastructure check against a same-job baseline; historical across-revision comparisons require retaining a baseline on a compatible host. The initial repository evidence summary is linked from #20.

## Initial local calibration — 2026-09-10

[Retained samples, distributions, configuration and evidence hashes](measurements/r0-initial.json) record the full successful local verification: **36 VM boots**, three validated warmups and fifteen measured repetitions. The unchanged control passed all 17 budgets. A real 100-tick service pause increased the read median from 0.0196 s to 1.0049 s, above the pre-established 0.0696 s limit. No sample was dropped or threshold widened.

All timings below are seconds and include host/transport overhead. The boot maximum illustrates why the median must not be presented as a worst-case guarantee.

| Metric | Baseline median | Baseline min–max | MAD | Control median | Upper limit |
| --- | ---: | ---: | ---: | ---: | ---: |
| `boot_ready_seconds` | 3.8662 | 3.5820–10.3339 | 0.2842 | 3.7855 | 5.7993 |
| `read_seconds` | 0.0196 | 0.0114–0.0206 | 0.0010 | 0.0203 | 0.0696 |
| `pressure_write_seconds` | 0.0492 | 0.0434–0.0932 | 0.0058 | 0.0670 | 0.0992 |
| `pressure_control_seconds` | 0.0047 | 0.0046–0.0059 | 0.0002 | 0.0056 | 0.0547 |
| `revoke_seconds` | 0.0136 | 0.0123–0.0142 | 0.0006 | 0.0141 | 0.0636 |
| `stopped_control_seconds` | 0.4007 | 0.3943–0.4129 | 0.0013 | 0.3999 | 0.6010 |
| `interrupt_seconds` | 0.2095 | 0.2087–0.2108 | 0.0003 | 0.2103 | 0.3143 |
| `admitted_control_seconds` | 0.0115 | 0.0112–0.0121 | 0.0002 | 0.0114 | 0.0615 |
| `drain_seconds` | 2.0597 | 2.0305–2.0671 | 0.0000 | 2.0600 | 3.0895 |

Across all three batches, the native counters stayed at 16 kernel page-table frames, 65,536 allocator metadata bytes, 9,608 manager metadata bytes, 200 fixture peak frames, 111 resident runtime frames and 58 additional frames at the C/H pressure checkpoint. The page-rounded static ELF load was 651,264 bytes. These categories have different boundaries; do not add them into a claimed total RAM footprint. Every repetition checked recovery of frame/process/channel/I/O state and the exact independently decoded final file set.

This local record was collected from the working tree based on `2d53153fc230a3b0894c35c68d8cd03b413f7f8a`, with the changes identified in its source status. Its actual guest build ID is `1c48cfd9881af425` and its frozen kernel SHA-256 is `7ab51cc876088e440e7aa590db934997a300d4038a8cced5bb020c5cbc7b1f55`. It is not mislabeled as a clean build of that base commit. Publication CI separately rebuilds and checks the published revision.

The preliminary `r0-first` collection was rejected after a native survivor-progress assertion panicked; its failed attempt remains in the local artifacts. A timer interrupt can occur on entry to user mode before the fixture advances its loop. The fixture now waits for both a preemption and actual user progress within the same 16-event budget, preserving both assertions and all isolation/reclamation checks. The final fresh collection passed all 18 kernel diagnostic boots. The pre-release harness also overstated the failed collection as 36 VM starts when it had started 35; counters now increment on actual process creation, and failed/preflight samples retain their actual counts.

Implementer review plus two additional code reviews addressed evidence completeness, file-object reclamation, environment provenance and the fixture assumption. Local validation passed 85 Rust tests, configured formatting/Clippy/native builds and 75 Python tests. This is not an independent security audit.

## Extending the baseline

Before adding a resident service, record its process/channel/handle/memory costs, add an effect-verified workload and preserve owner progress under its queue pressure/failure. Version the workload and establish a separate baseline when its meaning changes. Networking adds bounded local-fixture RTT/bytes and loss recovery; GUI/browser add frame/input and memory measurements for versioned scenes; the pilot adds operation success, denial/conflict/retry and provider-budget accounting. Capability adaptation in #38 consumes reviewed budgets; it must not reinterpret a single timing sample as an automatic policy decision.

Peak user-stack instrumentation, larger latency samples, hardware profiles and any production targets remain future measurement extensions. This initial harness can close #20 without implementing future services. It does not close those services, H1 as a whole or the wider v0.1 missions.


## Native read calibration — 2026-09-10

[Retained native-read samples and comparisons](measurements/r0-read-v1.json) record a fresh successful 36-boot verification of the stable-reference/range-read increment. All three warmups and fifteen measured repetitions passed. The unchanged control passes all 17 budgets; the real 100-tick service delay raises the read median to 1.0133 s, above the pre-established 0.0753 s limit. No sample was removed and no limit was widened after observing the control.

| Read round trip | Median | Min–max | MAD |
| --- | ---: | ---: | ---: |
| Baseline | 0.0253 s | 0.0233–0.0323 s | 0.0020 s |
| Unchanged control | 0.0298 s | 0.0208–0.0321 s | 0.0022 s |
| Injected delay | 1.0133 s | 1.0052–1.0204 s | 0.0063 s |

The same small-file workload now exercises the shell's checked native range read, including reference resolution, version pinning and SHA-256 validation. It does not measure 1 KiB throughput. Boot-ready baseline median is 3.6078 s (3.5006–3.9029 s); these five observations remain reference-VM measurements, without a hardware or worst-case guarantee.

The image packager changed to preserve the added dependency notices, changing the harness configuration hash. The automatic historical comparison correctly returns `incomparable`; its result is retained. This is a separate baseline, not a passing comparison against the initial timing budgets. The following counters show the reviewed resource growth relative to the retained initial report:

| Counter | Initial report | Native read | Difference |
| --- | ---: | ---: | ---: |
| Static kernel load reservation, including embedded programs | 651,264 B | 716,800 B | +64 KiB |
| Resident runtime | 111 frames | 120 frames | +9 frames / 36 KiB |
| Sampled C/H pressure growth above resident runtime | 58 frames | 68 frames | +10 frames / 40 KiB |
| Process diagnostic peak | 200 frames | 200 frames | unchanged |
| Allocator / manager metadata | 65,536 / 9,608 B | 65,536 / 9,608 B | unchanged |
| Reported kernel page-table frames | 16 | 15 | −1 frame |

The added service/SDK hashing and typed reference/range handling increase the linked native programs. This growth is accepted for the delivered checked-read contract within the unchanged eight-process/eight-channel and 64 KiB stack bounds. No kernel memory implementation or quota was changed; the page-table count is a measured layout-sensitive value, not a claimed MMU optimization. These accounting categories overlap and must not be added as a total. Stack high-water usage remains unmeasured. Reclamation and owner progress pass in every repetition.

Measured guest identity: build `106b9b6c3577ea54`, ELF SHA-256 `e4ee53c682ef965cb1f93a4276ce8d9d371354a95547b152b647a8f510112f76`, configuration `bccd309574387541f15feeef0930d215115508f6c4abfc1b5f180accc69bea7e`. Source was the explicitly recorded working tree over `212acb9`; these binary hashes identify the actual code. The exact same ELF passed the direct native read/recovery suite. Publication CI retains its own separate calibration.
