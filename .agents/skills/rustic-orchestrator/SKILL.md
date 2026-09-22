---
name: rustic-orchestrator
description: Coordinate nontrivial RusticOS development with Opus high orchestration, DeepSeek high implementation, and Astra low review. Use for bounded implementation, cross-module debugging, or independent review; skip trivial edits and simple questions.
---
<!-- SPDX-License-Identifier: Apache-2.0 -->

# RusticOS orchestration

Apply AGENTS.md and the user's current scope. This skill changes development workflow, not the OS architecture.

## Select and delegate

- Root owns decisions, integration and final verification. The configured root is claude-subscription/claude-opus-5 / high through the installed Codex Router; a running task may have a user override.
- Worker uses deepseek/deepseek-v4.1-flash / high through Codex Router. Explorer, tester and researcher retain gpt-5.6-luna / max. Reviewer retains gpt-6-astra / low. If the worker route returns a concrete provider-quota/unavailable error, report it and use gpt-6-luna / max for the already-authorized bounded implementation; this is explicit manual fallback, not silent or automatic failover. Record the model that actually completed the work. Do not change the default worker route. Other fallback models require owner selection.
- For nontrivial work, delegate a concrete implementation or independent evidence/review task when it has a useful boundary. Do not instantiate every role mechanically. Limit concurrent children to two.
- Give each child one objective, exact ownership, relevant source references, constraints, acceptance commands and expected output. Start with fresh minimal context when supported; do not fork the complete project conversation by default.
- For tools requiring explicit model/effort, pass the selected role values. A configuration file alone does not prove which model actually ran. If delegation is unavailable, report it and continue useful authorized work directly.

## Execute and verify

- One writer per file/subsystem. Workers report architecture, authority, ABI and dependency decisions before expanding scope; the root resolves them within the user's authorization.
- One owner runs tests that share build outputs, QEMU fixtures or volumes. Use disposable data; never use artifacts/terminal/data.raw for experiments.
- Retain the process/session identifier and deadline for long commands. Inspect bounded output, stop only owned processes on timeout, and do not leave hidden test jobs running.
- Root checks the diff and coordinates the required host/guest verification once. Repeat only for changes, failures or unresolved coverage.
- Use the Astra low reviewer for material correctness, unsafe, authority, concurrency and persistence changes. Review findings need source evidence; model choice is not proof of correctness.
- Children return a short report: changed files/symbols, result, commands/evidence paths, findings and blockers. Keep complete logs out of the root context.

## Continue efficiently

Use docs/WORK-STATE.md as the compact entry point. Check git status and the referenced active issue; retrieve deeper history only for a specific uncertainty. At a completed increment or handoff, update the revision, next acceptance target, tested evidence and remaining blocker. Do not mark a target complete without evidence.

Record measured usage when available, together with time, verified outcome and rework. Do not infer quota savings from raw tokens or a model name. Account-wide concurrent work can confound quota changes.

The user may override this workflow. Never claim a configured model, a test or a completed delegation without observed evidence.
