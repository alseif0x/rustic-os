<!-- SPDX-License-Identifier: Apache-2.0 -->

# Development orchestration

The project profile uses one role topology: a medium-effort root for coordination, maximum-effort explorer/worker/tester/researcher, and a low-effort independent reviewer. At most two child agents run concurrently. The owner selected this profile; it is not a measured cost or quality guarantee.

| Role | Codex | Claude Code |
| --- | --- | --- |
| Root / orchestrator | Astra — medium | Fable — medium |
| Explorer, worker, tester, researcher | Luna — max | Opus — high |
| Independent reviewer | Astra — low | Fable — low |
| Concurrent children | 2 | 2 |

Codex configuration is in [.codex/config.toml](../.codex/config.toml), with explicit models/efforts in [.codex/agents](../.codex/agents), and the [Codex rustic-orchestrator skill](../.agents/skills/rustic-orchestrator/SKILL.md). Claude Code configuration is in [.claude/settings.json](../.claude/settings.json) and [.claude/agents](../.claude/agents), with the [Claude rustic-orchestrator skill](../.claude/skills/rustic-orchestrator/SKILL.md). Both define bounded delegation, file ownership and verification. [WORK-STATE.md](WORK-STATE.md) is the compact continuation entry point. Preserve the repository's modularity and authority rules.

## Codex profile

Open a new Codex task/session rooted in this trusted repository to load project defaults. An existing task or explicit model/effort selection may retain its own overrides; writing configuration does not hot-switch the current task. Inspect the selected model and actual child trace before reporting which model ran.

For a nontrivial implementation, ask Codex to use the project workflow or invoke `$rustic-orchestrator`. The root may delegate only the useful roles. Other coding agents must not pretend to have invoked Codex tools they do not possess.

Project configuration does not replace global MCP/provider settings or change root approval/sandbox preferences. Explorer, researcher and reviewer request read-only sandboxes; live parent overrides may take precedence. Instructions and role names are not independent security boundaries.

## Claude Code profile

Role definitions live in [.claude/agents](../.claude/agents); each file pins its own `model` and `effort`, so a child keeps the agreed choice without the root repeating it. Invoke the workflow with `/rustic-orchestrator` or by asking for the project workflow, then delegate with the `Agent` tool and `subagent_type` `explorer`, `worker`, `tester`, `researcher` or `reviewer`. These start from fresh minimal context; `fork` copies the whole root conversation and is not the default.

The root model and effort are set project-wide in [.claude/settings.json](../.claude/settings.json), the counterpart of the root keys in `.codex/config.toml`. It carries only `model` and `modelSettings`; permissions, MCP servers and other integrations stay in each user's own settings. A session started before the file existed keeps its current selection; switch it with `/model fable` at medium effort, or start with `claude --model fable --effort medium`. A personal override belongs in an ignored `.claude/settings.local.json`.

Claude Code has no per-agent sandbox mode. Explorer, researcher and reviewer are restricted to read-only tools instead, and their instructions forbid writes; that is a narrower tool set, not an enforced sandbox. Role names and instructions are not independent security boundaries.

## Work and evidence

Choose one issue acceptance increment. Give a worker minimal source context and exclusive ownership; reserve a tester for shared build/VM artifacts. Keep native evidence distinct from host checks. Save full logs to ignored artifacts and return concise results. Review the final diff, address material findings, and update the compact work state.

Measure comparable completed increments before claiming savings: model/effort actually used, useful outcome, wall time, available usage metrics and rework. Do not equate total tokens with subscription quota, or attribute account-wide usage to one task while other sessions run. No billing credentials or API service is required by these configuration files.

## Provenance and maintenance

Inspired by [donvito/codex-astra-luna-orchestrator at 575e74e](https://github.com/donvito/codex-astra-luna-orchestrator/tree/575e74ebcf9b199513151a8996665a71cf64ce50), reviewed 2026-09-13. These are newly written RusticOS instructions/configuration using its agreed model topology. No upstream installer, skill text, role implementation or usage script is vendored or executed. No guest/build runtime dependency is introduced.

The adaptation uses a shorter project-specific workflow, two concurrent children, inherited root permissions, explicit shared-test ownership and a maintained work-state entry point. It preserves Luna max and Astra low review. Refer to [official custom-agent documentation](https://learn.chatgpt.com/docs/agent-configuration/subagents) for configuration and precedence. Recheck compatibility when updating Codex.

The Claude Code profile was added on 2026-09-15 from the same topology. Verified then: the installed CLI documents the `fable` model alias and the `low|medium|high|xhigh|max` effort levels, and all five role files plus the skill parse with their intended `model`/`effort` frontmatter. Not verified: agent discovery in a live session, and that a Fable root or an Opus child actually ran. Confirm the selected model and the child trace before reporting which model executed.

Setup validation on 2026-09-13 used Codex CLI 0.154.0: strict configuration loading succeeded; app-server config/read resolved the project layer with Astra medium, Luna max defaults and two children; skills/list discovered the project skill. All five role files parsed with their expected explicit models/efforts. An actual Astra low subagent reviewed the configuration and instructions without material findings. This establishes configuration/discovery and review, not a Luna implementation benchmark or new guest behavior.
