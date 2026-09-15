---
name: rustic-orchestrator
description: Coordinate nontrivial RusticOS development with a Fable medium root, Opus high workers and a Fable low reviewer. Use for bounded implementation, cross-module debugging, or independent review; skip trivial edits and simple questions.
---
<!-- SPDX-License-Identifier: Apache-2.0 -->

# RusticOS orchestration (Claude Code)

Apply AGENTS.md and the user's current scope. This skill changes development workflow, not the OS architecture. It is the Claude Code counterpart of the Codex profile in [.codex](../../../.codex); the role topology is the same and only the models differ.

## Select and delegate

- Root owns decisions, integration and final verification. The configured root is Fable at medium effort, set in [.claude/settings.json](../../settings.json); a running session may have a user override, and `/model` decides what actually runs.
- Explorer, worker, tester and researcher use Opus at high effort. Reviewer uses Fable at low effort. These are declared in [.claude/agents](../../agents). Preserve these agreed choices; do not silently downgrade a worker or raise reviewer effort for kernel work.
- For nontrivial work, delegate a concrete implementation or independent evidence/review task when it has a useful boundary. Do not instantiate every role mechanically. Limit concurrent children to two.
- Give each child one objective, exact ownership, relevant source references, constraints, acceptance commands and expected output. Spawn with `Agent(subagent_type: "explorer" | "worker" | "tester" | "researcher" | "reviewer")`, which starts from fresh minimal context; do not use `fork` by default, since it copies the whole root conversation.
- The agent definition supplies model and effort. Pass the `model` input only to deviate deliberately, and say so. A configuration file alone does not prove which model actually ran.

## Execute and verify

- One writer per file/subsystem. Workers report architecture, authority, ABI and dependency decisions before expanding scope; the root resolves them within the user's authorization.
- One owner runs tests that share build outputs, QEMU fixtures or volumes. Use disposable data; never use `artifacts/terminal/data.raw` for experiments.
- Retain the process/session identifier and deadline for long commands. Inspect bounded output, stop only owned processes on timeout, and do not leave hidden test jobs running.
- Root checks the diff and coordinates the required host/guest verification once. Repeat only for changes, failures or unresolved coverage.
- Use the Fable low reviewer for material correctness, unsafe, authority, concurrency and persistence changes. Review findings need source evidence; model choice is not proof of correctness.
- Children return a short report: changed files/symbols, result, commands/evidence paths, findings and blockers. Keep complete logs out of the root context.

## Continue efficiently

Use [docs/WORK-STATE.md](../../../docs/WORK-STATE.md) as the compact entry point. Check git status and the referenced active issue; retrieve deeper history only for a specific uncertainty. At a completed increment or handoff, update the revision, next acceptance target, tested evidence and remaining blocker. Do not mark a target complete without evidence.
