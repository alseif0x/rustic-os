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
| `ref WORKSPACE_PATH FILE_PATH` | Resolve authorized native objects to stable workspace/resource references |
| `read-ref WORKSPACE RESOURCE VERSION\|- OFFSET LENGTH` | Read a bounded range through the shared SDK; print pinned version, range hash, epoch and exact bytes as hex |
| `echo TEXT...`, `status` | Print arguments; show the previous command's status |
| `run spin`, `run fault`, `run exit` | Preemptible utility, deliberate isolated invalid-instruction fault, or exit code 7 |
| `run read FILE`, `run probe FILE OTHER` | Read selected file; probe additionally verifies denial of the other file and privileged kernel/console calls |
| `run watch FILE [TICKS]` | Repeated reads in a separate utility; optional grant lifetime in PIT ticks (100 ticks/second) |
| `ps`, `kill PID`, `reap PID` | Inspect processes; terminate/reap only this shell's utility children |
| `permissions [PID]`, `revoke PID` | Inspect file scope/rights/generation/deadline/report; request a fence for the whole client/helper session; poll `revocation PID` for access and effect status |
| `session FILE OTHER [TICKS]`, `helper PID FILE OTHER`, `act PID ACTION`, `move-check C H` | Run the [deterministic client/helper authority mission](AUTHORITY.md); explicit subsets and shared revocation |
| `act PID api-read\|read-open\|read-next\|fill` | Run [native read diagnostics](FILES-READ.md), including a controlled pause between chunks; `fill` requires write authority |
| `actor-status PID`, `revocation PID` | Query pending/complete actor work or requested/unconfirmed/fenced access without waiting for files |
| `stall files TICKS` | Owner-only stopped-service diagnostic; 0 is indefinite, 1–1000 is bounded; [procedure and limits](TAKEOVER.md) |
| `services`, `mem` | Query service identities, owner-policy state, frame/process/channel counts |
| `restart files [async]`, `job-status [ID]` | End utility sessions, drain pending I/O and remount; optionally return a job ID immediately, then collect its fresh binding |
| `hold-io SKIP TICKS`, `io-status` | Owner-only [real-submission completion-observation diagnostic](FOREGROUND-CONTROL.md); skip 0–16 writes/flushes, hold for 1–500 ticks |
| `retry-key PATH KEY`, `replace PATH VERSION TOKEN TEXT`, `receipt ID TOKEN` | Prepare an explicit retry token, commit a tracked whole-file replacement, inspect its retained result |
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

A lost response or failed submitted mutation is an uncertain result. The new tracked replace/receipt API can recover the original result after fresh authorization; ordinary write remains untracked. See [recoverable replacements](FILE-RECOVERY.md) for tokens, two-record retention, explicit legacy-disk upgrade and error handling. Creating an absent file and writing its contents remain two commits. The complete logical service-v1 contract remains in #12/#22/#43.

`cat` and `read-ref` use the same native `files.read` SDK as deterministic clients. Stable references combine volume lineage with a selected workspace directory and object ID; they confer no rights and require a fresh binding after restart. Each read pins a version and verifies the SHA-256 of exactly the returned bytes. Only `cat` can fall back to the legacy reader when lineage/recovery metadata is unavailable; permission, version and integrity errors do not trigger fallback. See [the read guide](FILES-READ.md) for canonical reference syntax, manual examples and the distinction between observing an epoch and permission to inspect receipts.

## Verification

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/terminal_test.py
# Also included in the complete direct and isolated suites:
python3 tools/boot.py test --timeout 45
python3 tools/sandbox.py test --revision "$(git rev-parse HEAD)"
```

The terminal driver sends real UART input, checks normal/error output, editing/overflow, full object capacity, two simultaneous utilities, faults, unauthorized control, scoped reads, expiry/revocation, malformed-policy repair and repeated service restart. After fault/restart/reap cycles, frame/process/channel/pending-I/O counts return to the same baseline. A second QEMU process mounts the same disk. An independent Python reader checks metadata/data CRCs, persisted file and policy bytes, and untouched reserved sectors.

The native read procedure adds shared binary/text/EOF range vectors, read-only helper access, denial of receipt inspection, version changes and revocation between chunks, identity persistence across restart/reboot, and changed identity after remove/recreate. With the pinned contract environment installed, validate its logical range exchanges using `.cache/contracts-venv/bin/python -m tools.contracts read-native --evidence artifacts/terminal-test/terminal.json`. This consumes evidence from `tools/terminal_test.py`; it does not start a VM or establish the remaining seven catalog operations. [FILES-READ.md](FILES-READ.md) describes the complete bounded-range fixture profile and its limits.

Evidence includes serial transcripts, `terminal.json`, `files.bin` (the bounded 89,088-byte format area), image/kernel identities and runner results. Pure filesystem tests separately interrupt each write/flush with partial-sector cases and recover either the old or new complete file. Those model tests are not an exhaustive physical power-loss test or a production filesystem guarantee.

Review is by the implementing agent, with automated host/native checks; no independent audit. Hardware support, complete service-v1 operation semantics, broader authority/takeover semantics and observability remain tracked in #12/#13/#15/#22 and H2. The [measurement harness](MEASUREMENTS.md) supplies repeated native resource/control checks. The terminal increment establishes a working manual path without claiming the entire OS roadmap is complete.
