<!-- SPDX-License-Identifier: Apache-2.0 -->

# Durable admission and cancellation storage

This original increment supplies the storage prerequisite for #12/#13/#43: a replacement can acquire a persistent identity **before** changing a file, and a prevented replacement can retain an immutable cancellation result across restart. It builds on [publication ownership](FILE-PUBLICATION.md) and [owner control](FILE-CONTROL.md). Subsequent increments add [public cancellation](FILE-ADMISSION-API.md), [scheduled execution](FILE-SCHEDULING.md), [coherent observation](FILE-OBSERVATION.md) and [format-5 prevention causes](FILE-PREVENTION.md). The logical service-v1 `operations.cancel` endpoint remains pending.

## Identity and admission

`Volume::admit_replace` validates the workspace, file version, size and retry namespace, reserves one retained slot, and publishes the original arguments and up to 1 KiB of bytes. It returns `AdmissionStatus` only after the metadata header's final flush succeeds. It does not change the file. Submission errors remain `Uncertain`; lookup after remount can determine whether the admission was published.

`AdmissionId { lineage, number }` is a separate Rust type and namespace from the existing completed-operation identifier. Its number is the sequence of the **published admission transaction**, not a speculative future file version. Every admission, cancellation, file effect and epoch rotation advances the existing monotonic volume sequence. An acknowledged admission number cannot become a later operation's identity. Sequence exhaustion rejects new work before I/O. An unacknowledged failed admission has no promised identifier; recovery uses its retry key.

The retry namespace remains trusted subject + volume lineage + workspace root + epoch/key. An identical retained submission returns its original state without writing, including a cancellation or a receipt whose file was later deleted. Changed arguments conflict. Existing format-3 completed records acquire no invented admission identity: the admission API returns `Unsupported` for a matching older scoped record. Legacy volume-wide records retain their separate namespace.

The admission stores the original service instance and complete arguments, but no grant, endpoint or delegated authority. Both lookup forms require authorization by their caller. A subject passed to the pure storage crate is a trusted binding, not authentication by itself. Neither an identifier nor a retained record authorizes execution.

## States and settlement

| Durable state | Stored evidence | Explicit next action |
| --- | --- | --- |
| `Admitted` | Admission ID, original instance, retry namespace, object/version and bytes; no receipt | Reauthorize and execute, or cancel |
| `Cancelled` | Same identity/arguments and the terminal metadata sequence; no receipt | Read/replay only until explicit retention rotation |
| `Committed` | Same admission identity and the original atomic file/version/receipt transition | Read/replay only until explicit retention rotation |

`prepare_admitted` explicitly starts execution. It rechecks the current workspace and file version, then uses the same exclusively borrowed `Publication` and `Disk`/`PollDisk` mechanics as existing replacements. An intervening edit fails its precondition. A committed replay needs no surviving file and issues no I/O. A cancelled admission cannot execute.

An early `Publication::cancel` stops file publication after any pending command drains. It leaves the durable record **Admitted**. The caller must then run `cancel_admission`; only that transition's successful final flush establishes durable `Cancelled`. A crash in the gap retains admitted work without claiming cancellation. Once a header may have been submitted, the writer must settle; the file effect and committed admission state publish together. Cancelling an already committed admission returns its existing `Committed` state, never a rollback claim.

The borrowed writer prevents simultaneous cancellation through the same volume owner. Errors, forgotten guards, adapter unwind and abandonment during submitted I/O fence the live volume. Synchronous metadata commits now establish that fence before entering the disk adapter as well. Remount reads durable facts; **it never resumes admitted work or restores old authority**. There is no automatic retry or background recovery execution.

## Format, migration and bounds

Format 4 retains the existing 174-sector volume, two metadata banks, seven recovery sectors per bank, two global records, 32 objects and 1 KiB files. `RUSTREC3` identifies its recovery encoding. A record adds an admission number at bytes 64–71, terminal sequence at 72–79, and state byte at 80 (`1` admitted, `2` cancelled, `3` committed). Bytes 81–511 remain zero. Empty extension bytes preserve an older completed record; an uncommitted admission's internal committed-version field is zero and is never exposed as a successful receipt.

Decoding checks the version/magic, reserved bytes, sequence ordering, namespace, state/receipt agreement and duplicate retry or transaction identities. All of this state remains covered by the selected header's recovery CRC. Data, version and committed receipt still share one publication header. Admission and cancellation each use 14 write/flush commands; executing the admitted replacement uses the existing 17.

`enable_admissions` is an explicit one-way upgrade from an operations-enabled volume. It preserves files and all historical receipts, then publishes format 4 in both banks. A successful upgrade leaves no older-format bank for an old reader to select. Interrupted migration can be recovered and explicitly repeated. There is no downgrade path. Production provisioning and the shell do **not** automatically upgrade user disks to format 4; the new native fixtures use disposable volumes.

An admission consumes one of the same two global slots as legacy/completed receipts. Full capacity rejects before admission I/O. `advance_epoch` returns `Busy` while any admission is nonterminal; an accepted operation cannot disappear through routine history cleanup. After all retained admissions are terminal, explicit owner rotation atomically advances the volume-wide epoch and removes records. Old retry keys are then expired. These are shared volume limits, not independent workspace quotas, archival retention or offline rollback protection.

## Reproduction and evidence

Run with the pinned [development environment](DEVELOPMENT.md):

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode block-user --timeout 45
python3 tools/boot.py run --mode recovery-test --timeout 60
```

Nine new filesystem tests cover persistent admission/cancellation, historical replay, lost admission-response recovery, intervening edits, subject separation, shared capacity, epoch fencing, all admission/cancellation/effect/upgrade/retention write and flush cuts, torn sectors, adapter unwind and abandoned guards. Valid-checksum corruptions challenge the decoder's semantic checks, including state/receipt disagreement and colliding transaction identities. Those failure cuts are host disk models, not native device failure injection.

The native `block-user` scenario retains its two separate VMs and original sector/publication checks. Two additional sequential ring-3 test processes per boot exercise disjoint disposable volumes. The first retains one cancelled admission and one committed admission; the second retains admitted but unexecuted work. The second VM checks the original identities/states, arguments, preconditions, conflicting retries, cancellation of terminal states and zero writes during replay/lookup. An independent Python CRC/content/version/record reader verifies both volumes after each VM and requires their complete selected-sector hashes to remain unchanged. Four test applications are launched and reaped sequentially per boot; resident process, block-handle, DMA, stack and per-application event limits are unchanged.

Development exposed a ring-3 stack fault at `0x7ffefd78`, below the 64 KiB application stack. Separating fixture setup/replay buffers and preventing publication buffers from being inlined into unrelated dispatch frames removed that cumulative allocation. Combining all old and new fixture work in one process also exceeded the existing 4,096-event test budget; each new volume fixture has its own sequential process with the same budget. These failed runs remain development evidence, not passing validation.

## Review and next integration

Original code remains `no_std` and forbids unsafe in the filesystem crate. Admission types, queries, state transitions and encoding have separate modules. The shared fixture disk adapter owns only a bounded sector view; the kernel only schedules/reaps test processes. No new dependency, unsafe boundary, kernel policy, IPC opcode or larger wire payload is introduced. Review is by the implementing agent and automated checks, not an independent audit.

The [service control integration](FILE-ADMISSION-CONTROL.md) makes admission and terminal metadata writes pollable, binds fresh execution authority and persists terminal cleanup after owner revocation. The subsequent [public admission API](FILE-ADMISSION-API.md) adds the separate cancellation right, bounded status framing, actual IPC response-loss recovery, explicit restart behavior and selected native I/O failures. [Live control](FILE-ACTIVITY.md) and [explicit scheduling](FILE-SCHEDULING.md) now provide public in-flight cancellation and bounded background execution. [Format 5](FILE-PREVENTION.md) retains distinguishable prevention causes after explicit migration. These storage states do not establish full native/function/MCP conformance, physical power-loss guarantees or automatic execution after restart.
