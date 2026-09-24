<!-- SPDX-License-Identifier: Apache-2.0 -->

# Native session runtime extension 1

Implemented bounded runtime for #45 and the first #13/#14 terminal increment. Base process ABI 1.0, IPC 1 and block 1 remain unchanged. No caller identity argument grants authority.

## Capacity and execution

Eight process slots support supervisor, file server, shell and two utilities, with three bounded spare slots for recovery/negative fixtures. Eight IPC channels provide private supervisor-shell and supervisor-files control plus file-client channels; 32 handles, 16 per owner, retain two 64-byte messages per direction. User stacks are 64 KiB with an unmapped guard, allowing bounded service buffers/nested codecs while retaining the 256-page process limit. #20 receives measured occupancy and pressure results; these are implementation bounds, not general performance claims.

Newly spawned programs remain dormant until the authorized supervisor sets bootstrap arguments and starts them. Names/manifests are descriptive; the executable catalog is explicit trusted native ELF data. No host path or host command is accepted. File format/session policy stays in native services; the kernel supplies lifecycle, owned handles, memory and device mechanisms.

## Additive syscall convention

INT 0x80 keeps RAX as number/result and RDI/RSI/RDX as arguments. No Rust layout crosses the boundary.

| Number | Operation and bounded arguments | Result |
| --- | --- | --- |
| 15 | Clock/yield: zero arguments | Monotonic PIT ticks; scheduler resumes another ready process |
| 16 | Console write: pointer, 1–256 bytes, 0 | Written byte count; current console owner only |
| 17 | Console read: pointer, 1–64 bytes, 0 | Copied bytes or WouldBlock; validate full output before consuming input |
| 18 | Console wait: zero arguments | Block until input is available; retain no pointer |
| 19 | Wait set: pointer to 1–8 u64 IPC tokens, count, timeout ticks 0–1000 | 0 on readiness/timeout; invalid/closed handles wake with a typed error; pointers are copied before blocking |
| 20 | Supervisor control: in/out pointer, exactly 64 bytes, 0 | 64 bytes; current trusted supervisor only, full write range validated before effect |

The control packet is eight little-endian u64 words. Word 0 is the opcode. INFO=0 returns runtime version/ticks/free frames/capacity/occupancy. SPAWN=1 takes a trusted catalog identifier and returns a dormant child PID. START=2 takes child PID and three bootstrap integers. CONNECT=3 takes two owned child/self PIDs and returns their separate handles. BLOCK_GRANT=4 takes child PID, rights, first sector and sector count. CONSOLE_GRANT=5 assigns/revokes the exclusive console child. PROCESS=6 queries a bounded slot and returns PID/state/exit/preemptions/parent/catalog entry. KILL=7 and REAP=8 address owned children only. SHUTDOWN=9 requests clean session teardown. DEVICE=11 reports current block geometry. Unused request words must be zero. Supervisor selection is trusted boot state, never a packet field.

File-service scope generations are separately bound to the authenticated client endpoint/PID. A moved token cannot create a new service grant. Console ownership is independent from file rights: child output cannot write directly to the owner's prompt. Services wait on bounded endpoint sets; when all user processes wait, the kernel sleeps until the next timer interrupt and continues polling disk/input and deadlines. No permanently spinning control process is required for an idle terminal.

MOVE_ENDPOINT=13 takes source PID, source token, target PID and attenuated IPC rights. Only the trusted supervisor may call it, both processes must be live and supervisor-owned, and it returns the new target token while invalidating the old token. File-service identity and session roots are unchanged; [native authority acceptance](AUTHORITY.md) checks movement, denial and root death.

CLOSE_ENDPOINT=12 takes an owned PID/token to roll back trusted provisioning. It cannot close a foreign process's authority. Full-capacity loader/lifecycle tests cover all eight slots; the terminal deliberately limits utilities to two.

HOLD_COMPLETION=14 takes an owned live PID, a 0–16 mutating-submission skip and a 1–500-tick observation hold. OBSERVATION_STATUS=15 reads the bounded arm/held-request state. Both require the trusted supervisor and the sdk-test catalog configuration; ordinary utility/device grants are denied. The real device is notified before observation is withheld. See [the diagnostic and admitted-I/O proof](FOREGROUND-CONTROL.md).

## Staged native images (`sdk-test`)

The sdk-test runtime also exposes a single supervisor-owned image transaction through CONTROL. STAGE_BEGIN=16 takes the ELF byte length (64 bytes through 512 KiB), a user pointer to exactly 128 manifest bytes, and a feature allowlist. The allowlist must be a subset of the currently known application request bits. The supervisor supplies this admission policy; it does not grant any requested capability. Begin returns a monotonic transaction generation and the 4096-byte copy limit. STAGE_COPY=17 takes that generation, the exact next offset, a user source pointer and a length from 1 through 4096. STAGE_COMMIT=18 takes the generation and returns a dormant child PID; STAGE_ABORT=19 takes the generation and returns zero words.

The stage owns a contiguous kernel frame run sized to the declared image, capped at 128 pages. It is not a user heap allocation or a large stack array. Physical fragmentation can refuse a run even when aggregate free RAM is sufficient. The owner PID and generation are checked on every operation. Manifest schema-v2 fields and names, the supervisor's feature allowlist, exact transfer length, SHA-256 and the supported ELF subset are validated before process-slot or user-address-space allocation. The executable field is the admitted filename; the identity field must pass the manifest's exact ASCII identity syntax. Hash matching binds the bytes to the manifest but does not authenticate a publisher or storage source. The resulting child has the supervisor as parent and a diagnostic dynamic-image program marker; it has no endpoints, block grant, console grant or other capability unless the supervisor supplies one separately.

An invalid current transaction, incomplete transfer, bad source range, policy refusal, digest mismatch, abort or loader failure clears and erases the stage. A duplicate begin returns Busy while preserving the open transaction; a stale generation returns Invalid without affecting a newer one; an unauthorized caller cannot clear it. Supervisor shutdown and exit release the buffer. There is no inactivity timeout, so the trusted supervisor must abort an abandoned transaction or exit. This mechanism is compiled only into sdk-test images. The kernel transaction itself does not read V7 storage, install packages, select an owner shell program, authenticate publishers, or establish a production launch policy; its only caller is the supervisor job described next. The ordinary boot catalog remains the production source in this increment.

### Storage-sourced dormant images (`mode=terminal-v7`)

In the explicitly selected read-only V7 profile the owner can ask the supervisor to feed one stored ELF and its manifest into this transaction: `stage-ref WORKSPACE ELF MANIFEST ELF_VERSION MANIFEST_VERSION` sends the additive owner request `STAGE_V7=36` (`crates/abi/src/supervisor.rs`) and returns a job. The kernel ABI is unchanged. The supervisor reads with its own read-only owner client (scope: the whole workspaces tree), never with shell authority, and polls the reads without blocking owner control.

The pair is pinned by the owner, not discovered. The supervisor first reads the manifest as one range at the owner's manifest version and requires exactly 128 bytes ending at EOF. It then reads the ELF in 1,024-byte ranges at the owner's ELF version, opens the kernel transaction with the size learned from the first range, and copies each verified range in order before committing. Every range must name the same lineage and workspace, carry its pinned version and repeat the retry epoch observed with the manifest; ELF ranges must be contiguous, keep one size and end exactly at EOF. These checks live in the host-tested `rustic_supervisor::image_pair` module; the job in `apps/supervisor/src/work/stage.rs` owns the reader, the transaction generation and cleanup. Any refusal aborts an open transaction and drains an abandoned read before the job reports it. While pending, `job-status` shows phase `1` until the kernel transaction is open and phase `2` while ELF ranges are copied. A completed job reports `[0, pid, elf_version, manifest_version]`; a refusal reports a `stage` status: `9` pair mismatch, `10` image size, `32 + e` a file-service error (a stale pin is `45`, `Version`), `64 + e` a kernel staging error, or `6` when `restart files` superseded the job (its transaction is aborted and the old owner client is replaced). `job-status` renders both. A commit reply without a child, or for another generation, is a service failure; a child returned with a mismatched generation is killed and reaped rather than tracked.

The supervisor admission policy for storage-sourced images is `image_pair::STORAGE_FEATURES = IPC | BLOCK`, the minimum that admits the shipped `file-server` manifest. Admission is not a grant: staging issues the child no endpoint, block, console or control authority and does not start it. The supervisor tracks one staged child at a time; further requests are busy until the owner has `kill`ed and `reap`ed it. `ps` shows it as program `staged`, and `permissions PID` reports zero scope/rights/expiry with the kernel generation, the staging duration in ticks (`report`), the image bytes (`bytes`) and the verified ranges (`other`).

The job has a measured budget of 15,000 ticks instead of the ordinary 1,000-tick owner deadline. Staging the 324,528-byte `file-server.elf` (317 ranges, about 8,600 exchanges) took 1,697-4,786 ticks (about 17-48 seconds) in eight stages across four guest runs under QEMU TCG on the reference machine; the cost is dominated by per-chunk sector reads in the service path, at a rate comparable to the shell's own `read-ref`. An expired job reports a timeout, aborts its transaction and leaves the supervisor degraded until `restart files`.

Known limits: the manifest's executable name is not bound to the V7 node name that supplied the ELF; the kernel checks only that the executable field is well formed and that the SHA-256 of the staged bytes matches the manifest. Range hashes and the manifest digest are transport and pair integrity, not publisher authenticity. Staging itself neither starts, installs nor grants authority to the child; starting is the separate request below. `python3 tools/v7_read_test.py` verifies, in the guest, two stale-pin refusals leaving no process, a successful dormant stage, `ps`/`permissions` inspection, the busy refusal while one is staged, and kill plus reap, both before and after a file-service restart, with the V7 volume digest unchanged. It also restarts the file service while a stage is in phase `2` and checks that the job reports `6` with no leftover process and that the next stage succeeds, which a leftover kernel transaction would refuse as busy. Both stale-pin refusals happen before STAGE_BEGIN. The other post-BEGIN cleanup paths (a pair mismatch or file error mid-ELF, a kernel copy/commit refusal, deadline expiry, and draining a reply still in flight on the same owner client) are implemented, but no guest case exercises them. The host tests cover the pairing decisions in `image_pair`, not the reader/transaction cleanup in the guest-only job.

#### Starting the staged child (control-only)

`start-staged PID ROLE` sends the additive owner request `START_STAGED=37` (words `[37, pid, role, 0, ..]`) and answers immediately; the kernel ABI is unchanged and the kernel's own START rule (supervisor caller, owned, dormant child) is the only mechanism used. The supervisor applies a separate storage launch policy, host-tested in `rustic_supervisor::storage_launch`, to the manifest facts (identity, requested features, version) the stage job kept from the exact 128 bytes the kernel admitted: the identity must be `rustic.utility`, the role must be one that needs no file, block or console authority (`exit`/FINISH, `fault`/FAULT, `spin`/SPIN), and the manifest must request `ipc`, which the issued channel implies. Everything else is refused with a `launch` status: `11` identity, `12` role, `13` features, `14` already started, `64 + e` a kernel refusal of the channel or the start; `2` means the PID is not the staged child. The shell forwards `read` and `session` too, so that refusal is the supervisor's decision rather than the shell's.

The only topology is control-only, the policy described in [authority](AUTHORITY.md#storage-sourced-launch-topology): one new channel between the supervisor and the child carries the role message and the child's report. The child starts with data token `0`, its control token and file-service PID `0`. It stays in the single storage slot, so it never takes one of the two utility slots, and `kill`/`reap` keep working through the staged-child path. After the start `permissions PID` reports zero scope, rights and expiry, the kernel generation, and the first three report words (`report`, `bytes`, `other`) instead of the staging facts; `reap` returns the exit kind and code and closes the supervisor's end. A refusal before the kernel start leaves the child dormant without an endpoint, and a channel opened for a refused start is closed on both ends. The kernel's CONNECT reports `Full` for any refused channel, including one to a child that is no longer live.

The utility reports its compile-time build tag (`RUSTIC_UTILITY_TAG`, default `0`, decoded by `rustic_utility::build_tag`) in word 1 of its FINISH report, and in nothing else. `python3 tools/v7_launch_test.py` builds the `terminal-v7` image once and copies it aside, builds two utility variants with tags `1` and `2` in separate cargo invocations and target directories (`target/v7-launch/utility-tag-N`, never `target/native`, whose utility is the one embedded in the kernel), seeds each into its own fresh V7 volume and boots the same frozen image once per volume. Each boot stages the pair, is refused `read` and `session` with status `12` and a non-staged PID with `2` while the child stays dormant, starts it with `exit`, is refused a second start with `14`, observes exit `1`/`7`, reads `report=7 bytes=TAG` and reaps code `7`, with process and channel counts back at their baseline. The first boot also kills a freshly staged child before starting it, which the kernel refuses as `Full` without leaking a channel, and still reaps it as killed; the second fills both utility slots with `run spin` (a third is busy and `run read` fails with `Unsupported`), then starts a staged child with `fault` as the seventh of eight channels and reaps vector `6`, and channel counts return to their baseline after the utilities are reaped. The harness checks that the image digest and the kernel inside it (extracted with `mcopy`) match the image metadata in both boots, that the variant ELF digests differ from each other and from the embedded utility, that the embedded utility artifact is unchanged by the variant builds, and that both volumes are unchanged; evidence is `artifacts/boot/terminal-v7-launch/terminal-v7-launch.json`. Staging the 131,360-byte variant (129 ranges) took 1,037-1,698 ticks in four recorded FINISH-case stages across two harness runs.

"Independently built" here means separate build invocations from the same source, differing only in the tag, with distinct ELF digests and distinct observable behaviour; it does not mean separate source trees, toolchains or publishers. `python3 tools/v7_read_test.py` also asks to start its staged `file-server` child and checks the identity refusal (`11`) leaves it dormant. Not exercised in the guest: real channel-table exhaustion, which the V7 profile cannot reach. The shell cannot name a file there (V7 path resolution is `Unsupported`), and the supervisor launches file-access utility roles, the only ones with a second (data) channel, only in the V5 profile (`administrative_ready`), so the peak is four resident channels, two control-only utilities and this start: seven of eight. Also not exercised: the features refusal (`13`), which no shipped utility manifest triggers; the host tests cover it.

#### Executable rollback on one migrated volume

Here "executable rollback" means only this: on unchanged data and an unchanged
kernel image, the owner stages and starts an older, already published
executable pair after a newer one. It is a selection, not a write, and it is a
separate operation from data migration:

| Step | Where | What changes |
| --- | --- | --- |
| Data migration: `seed5-history` source, then `migrate7` into a new V7 image | host, once | a new V7 image; the v5 source is only read and never booted, and its SHA-256 is checked unchanged |
| Publishing executables: `add7` the tag-1 pair, then the tag-2 pair into `/workspaces/migrated` | host | the V7 image gains four files and four retained records beside the migrated ones, which stay unchanged |
| Rollback: stage and start tag 2, then stage and start tag 1 | guest, one boot | nothing: the volume digest, `report7` and the `oracle7` view are identical before and after |

`python3 tools/v7_launch_test.py`, after its two launch boots, runs this on the
same built `terminal-v7` image (`tools/terminal_support/v7_rollback.py`). It
freezes the image again and requires its digest and that of the kernel inside it
to match the image metadata, and the launch and rollback runs to report the same
image and kernel digests. Tag 1 is published first, so every tag-1 file has a
lower V7 version than every tag-2 file. In one boot the owner runs the FINISH
case above for tag 2 (report `bytes=2`, exit `1`/`7`, reap), then for tag 1 by its
older pins (report `bytes=1`), each time with the refusals, the second-start
refusal and process/channel counts back at their baseline. The stage job's
completion echoes the pinned ELF and manifest versions, and the harness checks
that echo, but the echo is not the enforcement: the supervisor refuses any range
whose observed version differs from the pin (`image_pair`). The rollback claim
therefore rests on four checks: each `add7` answer's SHA-256 equals the digest of
the variant build it published; every tag-1 file has a lower V7 version than
every tag-2 file; the tag in each FINISH report comes from the ELF that is
running; and the volume is unchanged by the boot. Evidence is
`artifacts/boot/terminal-v7-rollback/terminal-v7-rollback.json`; in the first
recorded run the two 131,360-byte stages took 1,455 and 1,635 ticks.

Limits. Selection is owner-pinned for every stage: there is no persistent
"current version" pointer, no install or activation record, and nothing that
survives a reboot to say which pair is selected. Both variants declare the same
manifest identity and semantic version; "older" is publication order on the
volume. Pins and SHA-256 bind the ELF to its manifest and to what the owner
asked for; they do not authenticate a publisher. Only control-only roles of the
`rustic.utility` identity can run from storage, so this does not show rollback of
a service with file or device authority, and it does not show rollback of data:
data migration is one-way and needs a caller-owned copy of its source.

## Accounting and limitations

The normal topology is three resident processes and four channels: supervisor–files admin, supervisor–files owner data, supervisor–shell control and shell–files data. Two scoped utilities raise it to five processes and eight channels. In `mode=terminal-v7` a started staged child adds one process and one channel. Each program has at most 256 data/code/stack pages, including 16 stack pages; page tables are separately accounted. Pending wait sets copy at most eight tokens. Block capacity remains two request records/four grants, with one active device operation and three DMA frames.

Runtime errors use `u64::MAX - 64 - index`: Denied, Address, Size, Invalid, Busy, NotFound, Full, WouldBlock, Closed, Protocol. Counts/tokens remain in their documented non-error ranges. PROCESS state values are empty=0, dormant=1, ready=2, running=3, blocked=4 and exited=5; exit kinds are normal=1, fault=2, killed=3. A normal exit reports its code; a fault reports its vector.

Terminal images currently use the sdk-test build feature to include the native catalog alongside acceptance probes. The non-catalog boot-image target remains checked separately. The sdk-test staging path accepts bounded native ELF bytes only through the trusted supervisor control packet; there is no syscall for choosing a supervisor identity or device. This is an explicit packaging boundary, not production image hardening.

The boot supervisor is the trusted capability root. Services implement owner/helper rules through their private channels; manifests are admission requests only. Recovery is explicit and bounded, not an automatic restart storm. A failed supervisor stops the native session. A filesystem mount failure preserves the image and reports a failed startup job. The shell starts first, allowing Ctrl-C to leave its mount wait and reach owner status/retry commands; offline repair is not implemented.
