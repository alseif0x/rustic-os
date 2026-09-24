<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native terminal

RusticOS now boots a native supervisor, file server and interactive shell as separate Rust processes in ring 3. The host runs QEMU and forwards keyboard bytes; parsing commands, permissions and file operations execute inside RusticOS. A model and network are not required.

## Start and resume

Inside the reference Ubuntu 24.04 environment (including WSL2), from the repository root:

```sh
source ~/.cargo/env
python3 tools/terminal.py --initialize  # First launch: create a NEW dedicated disk.
python3 tools/terminal.py               # Later launches: mount the same disk.
```

Run one command at a time. The launcher builds the applications/kernel, packages a read-only UEFI boot image and attaches `artifacts/terminal/data.raw`. Initialization uses exclusive file creation and refuses an existing file. Normal launch refuses a missing, linked, wrong-size or concurrently used image. It never chooses a physical disk. The 4 GiB logical data image is sparse; the receipt-capable format uses only 174 sectors. Do not include it in Git.

Type `exit` to stop cleanly. Ctrl-C cancels the current input line or interrupts a foreground file/job wait; it does not undo admitted effects. See [foreground control and recovery](FOREGROUND-CONTROL.md). Ctrl-U clears it; Backspace/Delete removes the last character. QEMU's Ctrl-A X is an emergency exit, without a guest shutdown acknowledgement. The stdio backend disables host signal handling so Ctrl-C reaches the guest. Console ownership belongs to the shell; a utility cannot write directly to its prompt.

## Commands

`help` shows the everyday file, navigation, process and system commands first.
Use `help advanced` for the complete command reference, including service
protocol diagnostics and fault-injection controls. Invalid help arguments return
a usage error without printing a partial reference. The command table below
includes both everyday and advanced operations.

```text
help
pwd
ls /
mkdir project
cd project
write notes "Hello from RusticOS"
cat notes
stat notes
run read notes
ps
permissions 4
reap 4
services
mem
cd ..
exit
```

Use the PID actually printed by `run`; 4 is only an example. `run read` receives read authority for the selected file and reports a byte count through the supervisor. Inspect it with `permissions PID`; it does not print arbitrary child output into the prompt.

| Command | Behavior |
| --- | --- |
| `pwd`, `cd PATH`, `ls [PATH]` | Navigate directories; absolute and relative paths, dot and parent components |
| `mkdir PATH`, `touch PATH` | Create a directory or empty file; existing objects are errors |
| `write PATH TEXT...` | Create if absent, then replace with text using the observed file version |
| `cat PATH`, `stat PATH`, `rm PATH` | Read, inspect metadata, remove a file or empty directory |
| `tasks list PATH` | List a validated task document through a separate native application with read authority for that file; see [Tasks](TASKS.md) |
| `tasks preview add PATH TITLE`, `tasks preview done PATH ID` | Show a complete candidate and source version without applying the edit; previews receive read authority only |
| `tasks enable` | Explicit one-time persistent storage upgrade for task writes |
| `tasks add PATH TITLE`, `tasks done PATH ID` | Apply an immutable native plan with retained identity and expected-version checks |
| `tasks recover` | Query the original retained intent and verify exact bytes; never automatically replay |
| `tasks forget INTENT_KEY` | Explicitly discard recovery evidence; does not cancel or undo an effect |
| `ref WORKSPACE_PATH FILE_PATH` | Resolve authorized native objects to stable workspace/resource references |
| `read-ref WORKSPACE RESOURCE VERSION\|- OFFSET LENGTH` | Read a bounded range through the shared SDK; print pinned version, range hash, epoch and exact bytes as hex |
| `stage-ref WORKSPACE ELF MANIFEST ELF_VERSION MANIFEST_VERSION` | `mode=terminal-v7` only: the supervisor reads the pinned ELF/manifest pair with its own read-only authority and stages a dormant child; returns a job for `job-status`. See [storage-sourced dormant images](NATIVE-RUNTIME.md#storage-sourced-dormant-images-modeterminal-v7) |
| `replace-pattern-v7 WORKSPACE RESOURCE VERSION EPOCH KEY SEED SIZE` | `mode=terminal-v7` only: stream a deterministic SIZE-byte pattern (at most 524,288 bytes) as a profile-2 tracked replacement; prints the receipt and `write-v7 size=SIZE ticks=TICKS`. An exact repeat replays the same receipt without writing. See [V7 tracked writes](FILES-V7-WRITES.md) |
| `replace-pattern-v7 ... SIZE cut CHUNKS` | `mode=terminal-v7` diagnostic: send CHUNKS chunks (fewer than the file needs), have the owner revoke and reissue this shell's file binding, and print `cut-v7 chunks=N bytes=B job=J old=OUTCOME new=OUTCOME` for the next chunk on the old endpoint and on the new binding. Never commits. See [owner revocation during a transfer](FILES-V7-WRITES.md#owner-revocation-during-a-transfer) |
| `operation-v7 OPERATION_ID`, `operation-v7 WORKSPACE EPOCH KEY` | `mode=terminal-v7` only: print a retained profile-2 receipt in the same format as the write, then `lookup-v7 size=SIZE ticks=TICKS`. See [receipt lookups](FILES-V7-WRITES.md#receipt-lookups) |
| `replace-pattern-v7 ... SIZE hold CHUNKS` | `mode=terminal-v7` diagnostic: send CHUNKS chunks, have the owner ask for retention maintenance while the transfer is open, abort the transfer and print `hold-v7 chunks=N bytes=B maintain=OUTCOME abort=OUTCOME`. Never commits. See [owner retention maintenance](FILES-V7-WRITES.md#owner-retention-maintenance) |
| `maintain-v7` | `mode=terminal-v7` only, owner job: reclaim every terminal retained record, free the snapshot sectors no live file owns and advance the retry epoch; prints `maintain-v7 previous=e_... epoch=e_... records=N sectors=S job=J ticks=T`, or `error: Busy` while a transfer, stage or unresolved admission is open. Later writes must name the new epoch; old-epoch retries and lookups are `ExpiredEpoch`. See [owner retention maintenance](FILES-V7-WRITES.md#owner-retention-maintenance) |
| `admit-pattern-v7 WORKSPACE RESOURCE VERSION EPOCH KEY SEED SIZE` | `mode=terminal-v7` only: stream the same deterministic pattern as `replace-pattern-v7` as a profile-2 staged admission and accept it without executing it; prints the `admission-v1 id=ad_... service_instance=si_... state=admitted terminal=0` status and `admit-v7 size=SIZE ticks=TICKS`. An exact repeat reports the retained admission in its current state without writing. Then use `admission`, `execute-admission`, `cancel-admission` and `observe-admission-v2` with the printed ID. See [V7 staged admissions](FILES-V7-ADMISSIONS.md) |
| `admission-v7 WORKSPACE EPOCH KEY` | `mode=terminal-v7` only: status of the shell's admission with that profile-2 retry identity; never executes it |
| `admit-pattern-v7 ... SIZE revoke SKIP TICKS`, `execute-admission-v7 ADMISSION_ID revoke SKIP TICKS` | `mode=terminal-v7` diagnostic: stream the admission (for `admit-pattern-v7`), arm the completion hold for the file service (`hold-io SKIP TICKS`), send ACCEPT or EXECUTE without waiting, wait until the publication's command is held, have the owner revoke and reissue this shell's file binding, and print `revoke-v7 held=0\|1 job=J old=OUTCOME ticks=T`. The job completes only after the service settled the publication. Then it prints the outcome read on the new binding: the admission status for EXECUTE (by ID), or for ACCEPT the status or error of the lookup by retry identity. With `SKIP` 0 the held command precedes the header, so the execution is recorded cancelled with cause `authority_lost` and the acceptance leaves no admission. See [owner control during a publication](FILES-V7-ADMISSIONS.md#owner-control-during-a-publication) |
| `start-staged PID exit\|fault\|spin` | Start that staged child if its manifest identity is `rustic.utility`, with one control channel and no file, block or console authority; `read` and `session` are forwarded and refused by the supervisor. `permissions PID` then shows its report and `reap PID` its exit code. See [starting the staged child](NATIVE-RUNTIME.md#starting-the-staged-child-control-only) |
| `echo TEXT...`, `status` | Print arguments; show the previous command's status |
| `run spin`, `run fault`, `run exit` | Preemptible utility, deliberate isolated invalid-instruction fault, or exit code 7 |
| `run read FILE`, `run probe FILE OTHER` | Read selected file; probe additionally verifies denial of the other file and privileged kernel/console calls |
| `run watch FILE [TICKS]` | Repeated reads in a separate utility; optional grant lifetime in PIT ticks (100 ticks/second) |
| `ps`, `kill PID`, `reap PID` | Inspect processes; terminate/reap only this shell's utility children or the one staged dormant child (`ps` program `staged`) |
| `permissions [PID]`, `revoke PID` | Inspect file scope/rights/generation/deadline/report; request a fence for the whole client/helper session; poll `revocation PID` for access and effect status |
| `session FILE OTHER [TICKS]`, `helper PID FILE OTHER`, `act PID ACTION`, `move-check C H` | Run the [deterministic client/helper authority mission](AUTHORITY.md); explicit subsets and shared revocation |
| `act PID api-read\|read-open\|read-next\|fill` | Run [native read diagnostics](FILES-READ.md), including a controlled pause between chunks; `fill` requires write authority |
| `actor-status PID`, `revocation PID` | Query pending/complete actor work or requested/unconfirmed/fenced access without waiting for files |
| `stall files TICKS` | Owner-only stopped-service diagnostic; 0 is indefinite, 1–1000 is bounded; [procedure and limits](TAKEOVER.md) |
| `services`, `mem` | Query service identities, owner-policy state, frame/process/channel counts |
| `restart files [async]`, `job-status [ID]` | End utility sessions, drain pending I/O and remount; optionally return a job ID immediately, then collect its fresh binding |
| `hold-io SKIP TICKS`, `io-status` | Owner-only [real-submission completion-observation diagnostic](FOREGROUND-CONTROL.md); skip 0–16 writes/flushes, hold for 1–500 ticks |
| `retry-key PATH KEY`, `replace PATH VERSION TOKEN TEXT`, `receipt ID TOKEN` | Prepare an explicit retry token, commit a tracked whole-file replacement, inspect its retained result |
| `enable-admissions` | Explicit format-4 activation after `enable-operations`; see [the admission API](FILE-ADMISSION-API.md) |
| `admit-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT` | Persist replacement arguments without executing them |
| `admission ADMISSION_ID` / `admission WORKSPACE EPOCH KEY` | Inspect a retained admission after fresh authorization |
| `execute-admission ADMISSION_ID` / `cancel-admission ADMISSION_ID` | Explicit execution or cancellation; terminal states remain immutable |
| `schedule-admission ADMISSION_ID` | [Schedule already durable work](FILE-SCHEDULING.md); return an activity snapshot before settlement |
| `admission-activity ADMISSION_ID` / `request-cancel ADMISSION_ID` | Query queued/active work or request a volatile stop under separate rights |
| `enable-operations` | Explicit one-way upgrade to the [workspace operation format](FILE-OPERATIONS.md) |
| `replace-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT` | Replace at an observed version; return the original operation identity and SHA-256 receipt |
| `operation OPERATION_ID`, `operation WORKSPACE EPOCH KEY` | Inspect a retained completed result after a lost reply, later edit or restart |
| `rotate-receipts` | Deliberately advance the retry epoch and expire the two retained receipts after reconciliation |
| `exit` | Request supervisor shutdown and release process/device resources |

Quotes preserve spaces; single quotes are literal and double quotes allow backslash escapes. Backslash escapes the following character; backslash-n/backslash-t encode newline/tab outside single quotes. At most 16 arguments and 1024 input bytes are accepted. Overflow, unsupported bytes or unmatched quotes discard the command rather than executing a truncated prefix. Commands and names are ASCII. File output escapes control bytes, including ANSI escapes; newline remains a line break.

The shell is an interpreter for a fixed command set. There are no pipelines, redirection, background command syntax, history, dynamic executable loading, POSIX compatibility or GUI terminal. Utilities come from a trusted compiled catalog. Adding one requires an explicit build/catalog change; filenames never select host programs.

## Files, authority and recovery

The four roots are `/system` (read-only), `/data`, `/config` and `/workspaces`. Limits are 32 total objects including roots, 31-byte component names, 1 KiB per file and two staged file replacements. The line limit includes the command and path, so the maximum one-line text is less than the file-format maximum. See [FILES.md](FILES.md).

The supervisor owns a private administrative channel; the shell owns a separate manual control channel. Each utility has its own identity, endpoint and explicit scope. Spin/fault/exit utilities receive no file authority. Scoped readers cannot use another file, raw disk, console or supervisor control. Revocation is acknowledged by the file service after earlier serialized operations; an already admitted commit can finish before that acknowledgement. Expiry is checked at request admission, not at every physical write. The system does not claim rollback of completed effects. Revocation and actor commands are asynchronous: command acceptance does not establish their completion. The prompt and `pwd` use the last validated directory path. [Stopped-service acceptance](TAKEOVER.md) verifies independent owner progress, late acknowledgments and explicit recovery; [foreground waits are interruptible and supervisor startup/provisioning/restart advance as jobs](FOREGROUND-CONTROL.md).

`/config/owner-policy` stores the bounded initial rule:

```text
rustic-owner-v1
helpers=explicit
```

A malformed policy disables new helper file grants while the manual shell can still inspect and repair files. Restore it with `write /config/owner-policy "rustic-owner-v1\nhelpers=explicit\n"` and `restart files`. Runtime utility grants never survive restart/reboot. Restarting the service ends all utility sessions and binds the owner to its new PID/endpoint. It does not silently resume an old task.

A lost response or failed submitted mutation is an uncertain result. [Workspace replacements and operation lookup](FILE-OPERATIONS.md) recover the original result after fresh authorization, with stable identities and SHA-256 receipts; the [legacy replace/receipt API](FILE-RECOVERY.md) remains supported. Both share two retained slots and an explicit volume-wide epoch. Ordinary write remains untracked, and creating then filling an absent file takes two commits. General asynchronous lifecycle and the complete service-v1 surface remain in #47/#22/#43.

`cat` and `read-ref` use the same native `files.read` SDK as deterministic clients. Stable references combine volume lineage with a selected workspace directory and object ID; they confer no rights and require a fresh binding after restart. Each read pins a version and verifies the SHA-256 of exactly the returned bytes. Only `cat` can fall back to the legacy reader when lineage/recovery metadata is unavailable; permission, version and integrity errors do not trigger fallback. See [the read guide](FILES-READ.md) for canonical reference syntax, manual examples and the distinction between observing an epoch and permission to inspect receipts.

## Verification

`select-lifecycle operations.get` and `select-lifecycle operations.cancel` retain
both reviewed profiles on the shell's current client. Use `inspect-selected ID`
or `cancel-selected ID` during execution without another discovery exchange.
Restarting files clears both selections and requires explicit selection again.
The [profile guide](FILE-NEGOTIATION.md) describes the same-client deterministic
mission and its diagnostic commands; selection describes support and grants no rights.

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/terminal_test.py
# Also included in the complete direct and isolated suites:
python3 tools/boot.py test --timeout 45
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

The terminal driver sends real UART input, checks normal/error output, editing/overflow, full object capacity, two simultaneous utilities, faults, unauthorized control, scoped reads, expiry/revocation, malformed-policy repair and repeated service restart. After fault/restart/reap cycles, frame/process/channel/pending-I/O counts return to the same baseline. A second QEMU process mounts the same disk. An independent Python reader checks metadata/data CRCs, persisted file and policy bytes, and untouched reserved sectors.

The native read procedure adds shared binary/text/EOF range vectors, read-only helper access, denial of receipt inspection, version changes and revocation between chunks, identity persistence across restart/reboot, and changed identity after remove/recreate. With the pinned contract environment installed, validate its logical range exchanges using `.cache/contracts-venv/bin/python -m tools.contracts read-native --evidence artifacts/terminal-test/terminal.json`. This consumes evidence from `tools/terminal_test.py`; it does not start a VM or establish the complete eight-operation catalog. [FILES-READ.md](FILES-READ.md) describes the complete bounded-range fixture profile and its limits.

Evidence includes serial transcripts, `terminal.json`, `files.bin` (the bounded 89,088-byte format area), image/kernel identities and runner results. Pure filesystem tests separately interrupt each write/flush with partial-sector cases and recover either the old or new complete file. Those model tests are not an exhaustive physical power-loss test or a production filesystem guarantee.

Review is by the implementing agent, with automated host/native checks; no independent audit. Hardware support, complete service-v1 operation semantics, broader authority/takeover semantics and observability remain tracked in #13/#15/#22/#47 and the relevant later milestones. The [measurement harness](MEASUREMENTS.md) supplies repeated native resource/control checks. The terminal increment establishes a working manual path without claiming the entire OS roadmap is complete.
