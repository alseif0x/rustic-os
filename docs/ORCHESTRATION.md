<!-- SPDX-License-Identifier: Apache-2.0 -->

# Development orchestration

The project profile uses Astra medium for coordination, Luna max for explorer/worker/tester/researcher, and Astra low for independent review. At most two child agents run concurrently. The owner selected this profile; it is not a measured cost or quality guarantee.

Configuration is in [.codex/config.toml](../.codex/config.toml), with explicit models/efforts in [.codex/agents](../.codex/agents). The [rustic-orchestrator skill](../.agents/skills/rustic-orchestrator/SKILL.md) defines bounded delegation, file ownership and verification. [WORK-STATE.md](WORK-STATE.md) is the compact continuation entry point. Preserve the repository's modularity and authority rules.

## Loading and use

Open a new Codex task/session rooted in this trusted repository to load project defaults. An existing task or explicit model/effort selection may retain its own overrides; writing configuration does not hot-switch the current task. Inspect the selected model and actual child trace before reporting which model ran.

For a nontrivial implementation, ask Codex to use the project workflow or invoke `$rustic-orchestrator`. The root may delegate only the useful roles. Other coding agents must not pretend to have invoked Codex tools they do not possess.

Project configuration does not replace global MCP/provider settings or change root approval/sandbox preferences. Explorer, researcher and reviewer request read-only sandboxes; live parent overrides may take precedence. Instructions and role names are not independent security boundaries.

## Work and evidence

Choose one issue acceptance increment. Give a worker minimal source context and exclusive ownership; reserve a tester for shared build/VM artifacts. Keep native evidence distinct from host checks. Save full logs to ignored artifacts and return concise results. Review the final diff, address material findings, and update the compact work state.

Measure comparable completed increments before claiming savings: model/effort actually used, useful outcome, wall time, available usage metrics and rework. Do not equate total tokens with subscription quota, or attribute account-wide usage to one task while other sessions run. No billing credentials or API service is required by these configuration files.

## Provenance and maintenance

Inspired by [donvito/codex-astra-luna-orchestrator at 575e74e](https://github.com/donvito/codex-astra-luna-orchestrator/tree/575e74ebcf9b199513151a8996665a71cf64ce50), reviewed 2026-09-13. These are newly written RusticOS instructions/configuration using its agreed model topology. No upstream installer, skill text, role implementation or usage script is vendored or executed. No guest/build runtime dependency is introduced.

The adaptation uses a shorter project-specific workflow, two concurrent children, inherited root permissions, explicit shared-test ownership and a maintained work-state entry point. It preserves Luna max and Astra low review. Refer to [official custom-agent documentation](https://learn.chatgpt.com/docs/agent-configuration/subagents) for configuration and precedence. Recheck compatibility when updating Codex.

Setup validation on 2026-09-13 used Codex CLI 0.154.0: strict configuration loading succeeded; app-server config/read resolved the project layer with Astra medium, Luna max defaults and two children; skills/list discovered the project skill. All five role files parsed with their expected explicit models/efforts. An actual Astra low subagent reviewed the configuration and instructions without material findings. This establishes configuration/discovery and review, not a Luna implementation benchmark or new guest behavior.
