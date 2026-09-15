<!-- SPDX-License-Identifier: Apache-2.0 -->

# Current work state

Updated 2026-09-15. Replace this checkpoint rather than appending conversation history.

## Direction and current increment

Own modular Rust OS; isolated native applications, human and agent clients sharing authority and semantics. MCP is optional. Multiple preempted processes currently execute on one CPU; SMP, independent application installation and physical-machine support remain unimplemented. [Issue #1](https://github.com/alseif0x/rustic-os/issues/1) and the [systems roadmap](architecture/systems-roadmap.md) own sequencing.

Current increment over `8478820` (uncommitted working tree on `claude-orchestration-profile`): **a second native semantic tasks client with parity and its scoped/revoked recovery matrix**. The shell's owner client mechanism was extracted into `crates/tasks-client` (`rustic-tasks-client`): `Authority` (file client) and `Relay` (supervisor exchanges) traits, `Record::{path,object}`, a `Candidate` whose bytes must show the effect the summary and edit claim, free `plan`, and `Client::{apply, apply_candidate, recover, forget, pending}`. The shell consumes it with byte-identical output. `apps/utility` runs a persistent `TASKS_OWNER` loop (`apps/utility/src/tasks/*`) over the same crate with `Record::object(journal)`; the shell hands it a plan with `tasks hand PID add|done PATH VALUE` and steps it with `act PID tasks-apply|tasks-status|tasks-recover` and `tasks-owner-*`. Planning stays in the READ-only tasks ELF. [Tasks](TASKS.md) documents the protocol, phases, reply layouts and error codes.

Authority: the file service grants one optional second file scope per root (`GRANT_SECOND_SCOPE` 38, strict disjointness, files only, dropped on derive, fenced together). Only role `TASKS_OWNER` (15) uses it, with rights 7 and subject = the journal object id, so shell (subject 1) and utility receipts never share a retry-key namespace; the supervisor requires `other > 1 && other != scope`. The launch is a pure `grant::Sequence` in the supervisor library: install, extend, activate, and on any post-install failure (refused or malformed extension, unusable accepted reply, expiry, dead helper parent, failed activation) an explicit `REVOKE [33, slot+2]` before the job reports; a refused withdrawal marks the supervisor degraded. [Authority](AUTHORITY.md) records the contract. No kernel change, no new `unsafe`, no allocation, no new file-service profile.

## Verified evidence (final tree)

`cargo xtask check` passed: 345 host tests, fmt, Clippy `-D warnings`, kernel builds, guest clippy for every app in `native` and for shell/supervisor/utility in `native,tasks-acceptance`, the gated supervisor library tests and `tools/tests` (build-feature selection, build-id separation, cut command gated).

Acceptance build `fb88fb6e4d24c34d`, kernel SHA-256 `1a113589325f43f47111b3db40b5a8b5222dc33819e46bc354ddbf969832eb3a`:

- `python3 tools/tasks_owner_test.py` (two boots, disposable volume): add/done parity byte-for-byte with the shell client on a peer document, already-done no-op without receipt or journal write, version conflict `13`, revoked grant `18`, `Full` `11` with the retained key reported and the journal cleared, sequence faults `105`, launch refusals (aliased journal, the shell's own record, missing journal, and a directory journal that the service refuses so the installed root is withdrawn and the slot is reused), deterministic cuts `1..4` (prepared and lost-journal retained without effect; lost-reply committed with one receipt under the journal subject and recovered by a successor without resubmission; conflict `13` conclusive), unimplemented selector refused, a bounded kill race tolerated either way, recovery `27` query-only, `forget` semantics `0/104/103`, counters restored to 3 processes / 4 channels / 0 pending I/O (peak 4 / 6), and an apply after a real reboot. Evidence `artifacts/tasks-owner-test/tasks-owner.json`.
- `python3 tools/terminal_test.py`: 2,646 phase-one commands, two boots, all prior cases plus the `tasks.owner` phase. Evidence `artifacts/terminal-test/terminal.json`, `result.json`.
- `python3 tools/tasks_write_test.py`: two boots, the shell client's full write matrix unchanged. `python3 tools/boot.py run --mode recovery-test` (49 cases, 98 boots) and `--mode block-user` passed earlier on the same sources.

Ordinary build `ae0ffc1a8e042437`, kernel SHA-256 `1d9d3faa0720bb26bf74d4cc84b88629344ff9fa7e674bb627a91f826e70c1fd`: `python3 tools/tasks_owner_normal_test.py` (one boot, `terminal-init`) rejects `tasks-owner-apply-cut` and `tasks-write-acceptance` as unknown commands and commits `tasks hand` add/done through the utility with receipts under the journal subject. Evidence `artifacts/tasks-owner-normal/evidence.json`.

Two independent Fable low reviews of the actual diff: the first found the dangling grant after a failed second exchange and the shared subject-1 key namespace (both fixed with tests that fail on the old behaviour); the second found the accepted-but-unusable reply leak (fixed) and no blocker. The terminal suite failed twice on this host with `OSError: Cannot allocate memory` while writing its transcript to the Windows mount; both reruns passed on the same build id. Remote CI and a fresh isolated-container run are not claimed.

## Next acceptance

Keep [#22](https://github.com/alseif0x/rustic-os/issues/22) open: general discovery and complete mission acceptance remain. The second client is real but is launched from the shell-driven supervisor over the same document format; it does not establish general discovery.

Known limits to carry: the utility child that takes cut 2 or 3 loses its file binding and cannot rebind (no supervisor authority), so resolution always comes from a successor; `Intent::request` requires the journal version to exceed the target's version, which a fresh journal against a well-edited document refuses only after retention; an expired launch whose first GRANT reply is still outstanding is drained without inspection (pre-existing, mitigated by `degraded`); an intent retained by an older tasks-owner child under subject 1 cannot be recovered by the new subject rule (pre-release, evidence regenerated).

Keep #47/#43 correctness work finite. Optional file-service semantic/profile expansion remains frozen. #48–#52 own memory/allocation, bulk data, workspace capacity and independent delivery. Review #53 concurrency ownership before expanding runtime assumptions. #54/#37 and #55 remain bounded probes; #56 research does not change the adopted kernel model.

## Workspace constraints

Never experiment on `artifacts/terminal/data.raw`. Preserve the existing owner modification of `LICENSE`; do not stage it incidentally. Commits are authored as the owner (`alseif0x`) with no AI attribution. Maintained docs remain English. Continue using the project orchestration skill: Opus high bounded workers, Fable low review, at most two concurrent children and one owner for shared build/VM tests. On this Windows host every build and QEMU run goes through WSL Ubuntu; the terminal suite's host-side transcript write can fail with `ENOMEM` on the Windows mount and is retried.
