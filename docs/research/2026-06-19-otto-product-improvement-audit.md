# Otto Product Improvement Audit

Date: 2026-06-19

Scope: deep local audit of the current `otto_os` repo plus current external references for agentic developer tools, agent memory, MCP/tooling, and verifiable agent workflows.

## Evidence Base

Local evidence reviewed:

- Product README and architecture: `README.md`
- API and WebSocket contracts: `docs/contracts/api.md`, `docs/contracts/ws.md`
- Existing specs and plans: `docs/superpowers/specs/*`, `docs/superpowers/plans/*`
- Frontend navigation and sections: `ui/src/shell/Rail.svelte`, `ui/src/shell/App.svelte`, `ui/src/modules/*`
- Backend routes and composition root: `crates/otto-server/src/routes/mod.rs`, `crates/otto-server/src/modules.rs`, `crates/ottod/src/main.rs`
- Feature crates: `otto-sessions`, `otto-git`, `otto-product`, `otto-dbviewer`, `otto-swarm`, `otto-usage`, `otto-improve`, `otto-skills`, `otto-channels`, `otto-context`, `otto-rbac`
- Migrations and packaged assets: `crates/otto-state/migrations/*`, `crates/otto-product/assets/skills/*`, `crates/otto-swarm/assets/presets/*`

External research used:

- OpenAI Codex CLI is a local terminal coding agent that can read, change, and run code in the selected directory: https://developers.openai.com/codex/cli
- OpenAI recommends repository-level `AGENTS.md` files for project norms inherited with global defaults: https://developers.openai.com/codex/guides/agents-md
- Claude Code documents `CLAUDE.md`, auto memory, path-scoped rules, hooks for enforcement, and recommends concise project instructions: https://code.claude.com/docs/en/memory
- MCP tools expose external systems to models through named tool schemas: https://modelcontextprotocol.io/specification/2025-06-18/server/tools
- Anthropic notes that large MCP tool sets can bloat context and cost, motivating better tool discovery/execution patterns: https://www.anthropic.com/engineering/code-execution-with-mcp
- Cursor's agent guidance emphasizes typed systems, linters, tests, and verifiable goals as the signals agents need: https://cursor.com/blog/agent-best-practices

Verification performed:

- `cargo check --workspace`: passed.
- `cd ui && npm run check`: passed with 3 warnings:
  - `ui/src/modules/database/RedisKeyFilter.svelte`: `$state` captures initial `node`.
  - `ui/src/modules/database/TableDesigner.svelte`: `$state` captures initial `columns`.
  - `ui/src/modules/database/ResultsGrid.svelte`: unused CSS selector `.tb-note`.
- `cargo test --workspace`: all normal unit and integration tests passed, but final doctest phase failed in `otto-rbac` with `unresolved import otto_core::api::ApiTokenInfo`. The type exists in current `crates/otto-core/src/api.rs`, and `crates/otto-rbac/src/tokens.rs` imports it, so this needs build graph/doctest cleanup rather than a feature rewrite.

## Highest Leverage Direction

Otto already has a wide feature surface. The best next phase is not another large module. It is reliability, trust, evaluation, and interoperability around the existing modules.

The product has a strong differentiator: it treats agent CLIs as persistent, openable, terminal-native sessions and then layers workflows, product context, database tools, PR review, usage, and swarms around those sessions. The main risk is that the surface area is now ahead of the hardening layer: tests, docs, API contract completeness, secure automation boundaries, and live operational debugging.

## Cross-Cutting Platform

### Must Have

- Fix `cargo test --workspace` by resolving the `otto-rbac` doctest failure around `ApiTokenInfo`.
- Add a root `AGENTS.md` for this repo, with build/test commands, architectural ownership, and instructions for not damaging user work. Add `CLAUDE.md` importing `AGENTS.md` if Claude Code is a first-class target.
- Update `docs/contracts/api.md` for routes that exist but are missing from the frozen contract, including auth API tokens, API client, workflows, notifications, LSP, logs, filesystem, skill eval, insights, DB explorer, self-improvement, and provider update routes.
- Add a release checklist that requires `cargo check --workspace`, `cargo test --workspace`, `npm run check`, and ignored integration suites where infrastructure is available.
- Add a route inventory test that compares registered backend routes against `docs/contracts/api.md` to prevent silent contract drift.
- Split the largest files before more features land: `crates/otto-server/src/product_run.rs`, `crates/otto-server/src/modules.rs`, `ui/src/modules/product/OverviewTab.svelte`, `ui/src/modules/database/ResultsGrid.svelte`, `ui/src/modules/api/RequestBuilder.svelte`.
- Add a single diagnostic page or command that exports a support bundle: daemon version, DB migration version, provider detection, recent logs, failed sessions, active settings, and privacy-redacted route failures.

### Should Have

- Build a first-run health checklist: agent CLIs detected, auth state, Keychain accessible, database migrations applied, sidecar reachable, usage engine status.
- Add an internal "capabilities registry" so UI modules can hide or explain features based on provider/account/runtime availability.
- Add event replay tests for `/ws/events` so product, swarm, sessions, notifications, and activity stores remain stable.
- Add a schema/version compatibility gate for the SQLite DB and embedded ClickHouse usage store.

### Could Have

- Add a public plugin boundary for modules rather than every module being compiled into the daemon.
- Add local feature flags for experimental sections like Swarm and Workflows.
- Add built-in "demo workspace" seed data for screenshots, onboarding, and testing.

### Nice To Have

- Add in-app "What's new" generated from migrations and docs.
- Add a tiny benchmark dashboard for session startup, terminal attach, query latency, and route timing.

## Agents And Terminal Sessions

Current state: strong. Sessions are PTY-backed, resumable, split/tiled, searchable, attachable to issues/product stories, have handover, extra directories, prompt guard, activity trail, and terminal reconnect behavior.

### Must Have

- Add explicit session lifecycle states in the UI for "running", "waiting for user", "reauth needed", "suspended", "gone transcript", and "failed to resume".
- Make session replay/audit easier: one panel showing transcript path, provider session id, cwd, extra dirs, injected context, hooks/materialized files, and activity trail.
- Add automated tests for session restart/resume with `extra_dirs` and product/story attachment metadata.
- Add failure notices when prompt guard cannot satisfy a trust/auth prompt within a timeout.

### Should Have

- Add per-provider command preview before spawning or restarting a session, redacting secrets.
- Add "clone session" and "fork with context" commands to branch a working agent cleanly.
- Add terminal recording export for debugging and sharing.
- Add a read-only spectator mode indicator and explicit input-block feedback for viewers.

### Could Have

- Add session templates by workflow: bugfix, review, product analysis, support triage, database investigation.
- Add a task focus bar tied to current agent task list and status.
- Add a session "definition of done" checklist that agents can update.

### Nice To Have

- Add named pane layouts saved per workspace.
- Add quick compare between two sessions' latest outputs.

## Agent Swarm

Current state: active implementation exists across `otto-swarm`, `otto-state`, `otto-server`, UI store/pages, migration `0029_agent_swarm.sql`, and five preset YAMLs. It covers swarms, agents, projects, tasks, runs, shared board, lifecycle, recruiter, planner, scheduler, and live events.

### Must Have

- Finish hardening the current uncommitted swarm work before stacking more features: route contract, migration coverage, cancellation behavior, pause/resume semantics, and scheduler recovery after daemon restart.
- Add integration tests for a minimal swarm: create preset, create project, plan tasks, run one task, board ingest with session token, stop run, and abort swarm.
- Add guardrails for concurrency: per-swarm cap, global cap, per-provider cap, and "do not start if workspace has dirty forbidden paths" policy.
- Add durable coordinator state so active swarms recover after daemon restart without double-running queued work.
- Add explicit security boundaries for `otto-post` and `/ingest/swarm/board`: token rotation, expiry, and visible session-token status.

### Should Have

- Add a "run inspector" with prompt, cwd/worktree, injected skills, output file, parsed result, session id, retry count, and token/cost attribution.
- Add dependency/DAG validation in task edit flows, including cycle prevention and blocked reason.
- Add leader delegation visibility: parent task, delegated subtasks, review result, and handoff chain.
- Add manual approval gates for risky task categories before auto-submit.
- Add "single workspace scratch vs repo worktree" policy per project, not only per swarm/agent.

### Could Have

- Add swarm evaluation: compare a swarm run against a single-agent baseline for time, cost, pass rate, and review findings.
- Add reusable org templates editable in UI, not only packaged YAML presets.
- Add schedule calendar view for recurring agents.
- Add artifact registry per project for files, PRs, reports, screenshots, and run outputs.

### Nice To Have

- Add animated timeline playback of a swarm's board/runs.
- Add role-avatar customization without relying only on emoji/initials.

## Product: Jira, Confluence, Discovery, Planning

Current state: very strong and broad. Product supports story import, draft discovery, Jira full view, comments, transitions, assignee, attachments, tags, related stories, transcripts, analysis, questions, notes, rewrite, test cases, plan generation, history, inject bundle, learnings, and packaged product skills.

### Must Have

- Remove stale comments that call tabs placeholders in `ProductPage.svelte`; the tabs are now implemented, and stale comments mislead future agents.
- Add contract documentation for the complete Product API, including drafts/transcripts/publish-as-story/publish-as-RFC, plan generation, attach-product, and watcher routes.
- Add end-to-end product smoke tests with mocked Jira/Confluence: import, refresh no-change, add comment, ask/post questions, generate tests, approve, publish, inject into session.
- Add idempotency and dedupe checks around watcher reconciliation and comment posting, so repeat polls do not duplicate actions.
- Add visible "source freshness" and "last external sync" state per story.

### Should Have

- Add traceability matrix per story: requirements, open questions, test cases, generated tasks, related sessions, PRs, and decisions.
- Add story diff view between imported/current/rewrite/testcase versions.
- Add learnings governance: suggested vs accepted, source evidence, confidence, last used, disable reason.
- Add per-story provider/model/run settings saved as reusable presets.
- Add Confluence/Jira permission diagnostics when publish/comment/update fails.

### Could Have

- Add story-to-workflow conversion: build an Otto workflow from a story plan.
- Add story-to-swarm conversion: create a swarm project and task DAG from Product Plan.
- Add meeting/call transcript importers with speaker cleanup and decisions extraction.
- Add acceptance-criteria linting against INVEST/Gherkin/team-specific rules.

### Nice To Have

- Add a "PO dashboard" across stories: stale stories, unanswered questions, ready for dev, ready for QA.
- Add visual dependency map for related stories and linked issues.

## Git, PRs, And Review Agents

Current state: mature local git and remote provider support. Routes cover accounts, repo detect/add, status, branches, refs, fetch, diff, stage, discard, commit, push/pull, checkout, API collection sync, stash, merge/conflict flows, PR CRUD/comments/approve/merge/decline/commits, AI PR reviews, local reviews, handoff, and PR draft generation.

### Must Have

- Add safety rail for destructive git actions: discard, merge abort, branch checkout with dirty tree, and force-like operations should show exact affected files.
- Add provider failure diagnostics that separate auth failure, provider outage, rate limit, missing repo permissions, and malformed remote URL.
- Add tests for PR draft generation with staged/untracked/renamed/binary diff cases.
- Ensure review agents store enough evidence for every finding: file, line, snippet/hash, source agent, prompt config, and resolution state.

### Should Have

- Add review quality evaluation using the existing Skills Evaluator: seeded known-bug PRs, expected findings, false-positive tracking.
- Add "apply approved review comments" workflow that opens an agent with exact approved comments and target diff.
- Add merge readiness panel: CI status, approvals, unresolved comments, branch freshness, conflict status.
- Add local branch/worktree cleanup assistant.

### Could Have

- Add repository relationship graph across workspaces and linked Product stories.
- Add PR review profiles per repo/language.
- Add automated release-note generation from merged PRs and product history.

### Nice To Have

- Add visual commit graph interactions: cherry-pick, revert, branch from commit.
- Add inline blame/context around diff hunks.

## Database Explorer And Connections

Current state: strong native DB explorer for MySQL, Redis, MongoDB, ClickHouse, query editor, schema tree, structure view, saved/history, dashboards/widgets, JOIN builder, SSH/TLS support, inline editing, and agent explanation. Verification has deep parser/driver unit coverage and ignored E2E suites.

### Must Have

- Fix the three Svelte warnings in database components.
- Add a destructive-query protection policy with per-connection environment labels and configurable "prod requires typed confirmation".
- Turn ignored DB E2E tests into documented opt-in scripts with Docker prerequisites and CI labels.
- Add query cancellation tests per engine, not only UI affordance.
- Add audit log entries for write queries, inline edits, TRUNCATE/DROP generated tabs, and dashboard widget executions.

### Should Have

- Add query plan/explain support for MySQL, ClickHouse, and MongoDB where available.
- Add connection health badges and last error details in the DB sidebar.
- Add row-level edit diff preview before applying.
- Add schema snapshot/diff to compare environments.
- Add secret redaction verification tests for connection profiles, first commands, logs, and support bundles.

### Could Have

- Add saved query folders, variables, and parameterized query prompts.
- Add dashboard sharing/export.
- Add data masking profiles for PII-heavy tables.

### Nice To Have

- Add ERD generation from schema introspection.
- Add "ask agent about selected rows/schema" with structured context limits.

## API Client And Automations

Current state: API client includes collections, history, environments, automations, curl import, OpenAPI export, request builder, response viewer, daemon-side execution, cookies, redirects, TLS verification toggle, WebSocket/SSE/gRPC-related routes.

### Must Have

- Document the full API client contract in `docs/contracts/api.md`.
- Add secret handling for environment variables: mark sensitive values, hide by default, redact from history/logs/exports.
- Add execution safety for `verify_ssl=false`, cross-workspace cookies, and shared daemon cookie jar behavior.
- Add tests for curl import, variable substitution, auth modes, binary responses, too-large responses, redirects disabled, TLS disabled, and assertions.

### Should Have

- Add per-environment cookie jars instead of one daemon-global jar.
- Add pre-request scripts and test scripts with a sandbox boundary.
- Add collection-level auth inheritance.
- Add OpenAPI import, not only export.
- Add request diff/history restore.

### Could Have

- Add GraphQL schema introspection and query explorer.
- Add gRPC reflection browser tied into saved collections.
- Add load/smoke runs for collections with summary metrics.

### Nice To Have

- Add "generate API tests from this collection" into Workflows or Skills Eval.
- Add shareable examples with secrets stripped.

## Workflows

Current state: workflow editor and engine exist, with manual trigger, agent prompt, HTTP request, transform, delay, log, game_engine, verifier nodes. Natural-language generation falls back to trigger->agent if the LLM is unavailable. Game nodes are currently scaffolds.

### Must Have

- Mark scaffold node kinds clearly in UI and docs. `game_engine` and `verifier` should not look production-complete until an engine exists.
- Add workflow contract documentation and route tests.
- Add step-level retry, timeout, cancellation, and error policy in graph params.
- Add secrets/environment variable access model for HTTP and agent nodes.
- Add run retention and cleanup policy.

### Should Have

- Add typed node schemas with validation and UI forms generated from the schema.
- Add workflow-to-schedule support.
- Add workflow run event streaming instead of polling only.
- Add import/export as JSON for sharing and versioning.

### Could Have

- Add reusable subflows.
- Add conditional/router nodes and parallel branches.
- Add direct integration with Product plans, Git reviews, API collections, and Swarm projects.

### Nice To Have

- Add a workflow template gallery beyond the current game-oriented examples.
- Add visual run replay on the canvas.

## Skills, Context, Self-Improvement, And Skill Evaluation

Current state: strong foundation. There is a context library, skill/soul materialization for Claude/Codex, bundled skill library, product skill seeding, self-improvement engine with safe/queued edits and rollback, and skill evaluation UI/backend.

### Must Have

- Add repo-level `AGENTS.md` and optional `CLAUDE.md` bridge so context behavior matches current agent ecosystems.
- Add a policy distinction between guidance and enforcement. External docs emphasize that instruction files are context, while hooks/settings enforce behavior; Otto should make this explicit in UI.
- Add a "context preview" before session spawn: exact files, skills, souls, hooks, and generated AGENTS/CLAUDE content.
- Add tests that Codex and Claude materialization stay in sync with expected file conventions.
- Add skill versioning and changelog in the UI before install/update.

### Should Have

- Add path-scoped skills/rules, so large workspaces avoid loading irrelevant context.
- Add skill linting: frontmatter, trigger clarity, tool safety, missing examples, overly broad instructions.
- Add eval datasets per skill with expected outputs and failure cases.
- Add self-improvement provenance: which session caused the suggestion, exact evidence, risk classification, and rollback path.

### Could Have

- Add import from existing `CLAUDE.md`, `.claude/rules`, `.cursorrules`, `.windsurfrules`, and `AGENTS.md`.
- Add skill marketplace/source registry with trust levels.
- Add MCP tool descriptions as evaluable skills/context assets.

### Nice To Have

- Add skill "dry run" preview against a selected prompt/transcript.
- Add visual diff between installed and bundled skill versions.

## Insights, Usage, And Cost

Current state: embedded ClickHouse usage engine, transcript tailing for Claude/Codex, cost estimates, provider/day/session rollups, 4-way token breakdown, scheduled insights reports, and metrics.

### Must Have

- Add visible warning when usage attribution is incomplete because a provider transcript format changed.
- Add contract docs for usage and insights routes.
- Add regression corpus for Claude/Codex transcript parsing with real anonymized samples.
- Add cost model versioning and "unknown model" handling in UI, not only as zero/free.

### Should Have

- Add per-feature cost attribution: Product analysis, PR review, Swarm run, Workflow node, ad-hoc session.
- Add cost anomaly alerts and budget caps per workspace/provider/module.
- Add usage export as CSV/JSON.
- Add report regeneration provenance: source sessions, date range, providers, cache hit/miss.

### Could Have

- Add quality/cost scatterplots: review findings per dollar, product questions per run, swarm tasks per dollar.
- Add forecast based on recent usage.

### Nice To Have

- Add "why did this session cost so much?" drilldown.
- Add provider/model recommendation based on historical task type.

## Channels: Slack And Telegram

Current state: channel bridge exists for Slack/Telegram thread-to-agent sessions, with message/file relay, archiving, integration settings, channel session grouping, and channel transcript handling.

### Must Have

- Add explicit human approval mode for outbound customer/provider replies. Draft by default, send only when configured.
- Add redaction for secrets/PII before relaying files/transcripts into agents.
- Add retry/dead-letter queue for failed inbound/outbound bridge events.
- Add per-channel audit log: source message, agent session, outbound reply/file, status.

### Should Have

- Add ticket ownership/status mapping and auto-archive rules visible in UI.
- Add channel-specific prompt/context templates.
- Add file size/type policy and antivirus/security placeholder.
- Add Slack/Telegram credential health checks.

### Could Have

- Add Zendesk/Linear/GitHub issue bridges using the same channel abstraction.
- Add triage dashboard across open channel sessions.

### Nice To Have

- Add canned response library backed by skills.
- Add conversation quality review after closure.

## Settings, RBAC, Security, And Remote Access

Current state: local root onboarding, users, workspace roles, Keychain-backed secrets, loopback default, optional network listener, settings pages, notification center, daemon logs, provider settings, issue accounts, channels, LSP, self-improvement, usage, skills, context, and API token work in progress.

### Must Have

- Finish and verify API token routes/migration/docs/UI. Current source has `0030_api_tokens.sql` and token repo methods, but the contract and full route registration need review, and `cargo test --workspace` doctest currently fails.
- Add CSRF/origin strategy for any non-loopback/browser-served deployment mode.
- Add network listener hardening: explicit bind address, allowed origins, token requirements, rate limits, and warning UI.
- Add settings export/import with secrets excluded.
- Add backup/restore for SQLite state and context library.

### Should Have

- Add role matrix docs and tests for every major route.
- Add audit log for auth, settings changes, token creation/revocation, network listener changes, destructive DB/git actions.
- Add session/token revocation UI.
- Add secure remote access plan implementation from `docs/superpowers/plans/2026-06-17-remote-mobile-access.md`: daemon-served UI, tunnel-first recommendation, auth hardening, mobile layout.

### Could Have

- Add workspace-scoped service accounts.
- Add SSO/OIDC for team usage.
- Add managed policy file for enterprise deployments.

### Nice To Have

- Add QR login for local mobile access after remote access hardening.
- Add security posture score in Settings.

## UI Shell, Navigation, And Onboarding

Current state: compact rail, expanded navigator, command palette, plain-English orchestrator, global search for sessions, split/tiled sessions, right panels, notification bell, status bar, walkthroughs.

### Must Have

- Add onboarding path that creates a workspace, detects providers, installs/updates skills, and launches a first useful session.
- Add section-level empty states with "what to do next" actions for Swarm, Product, Git, API, Database, Workflows, Skills Eval, Insights, Usage.
- Add responsive/iPad baseline before remote access is promoted.
- Add route/module permission handling so viewer/root-only modules are clear.

### Should Have

- Add unified global search across sessions, stories, repos, PRs, connections, saved queries, workflows, reports, and swarms.
- Add command palette actions for the newer modules: create swarm project, plan product story, run workflow, open usage report, create API request.
- Add recent/recommended actions based on current workspace state.

### Could Have

- Add customizable rail ordering and hidden modules.
- Add a home dashboard with active sessions, risky blockers, recent reports, review queues, and stale stories.

### Nice To Have

- Add keyboard shortcut trainer/overlay.
- Add theme polish pass after functional hardening.

## Packaging, Docs, And Distribution

Current state: README has build steps, signing, DMG script, launchd plist, desktop sidecar layout, and warnings about experimental status.

### Must Have

- Add a deterministic local release script that runs checks, builds UI, builds `ottod`, copies sidecar, builds Tauri app, signs, and produces DMG.
- Add migration rollback/recovery docs.
- Add install/update docs for non-developers.
- Add troubleshooting docs for provider auth, Keychain, launchd, port conflicts, and corrupted DB.

### Should Have

- Add architecture diagrams per subsystem and a route/module ownership map.
- Replace `ui/README.md` Vite template text with Otto-specific frontend development docs.
- Add "known limitations" per major feature.

### Could Have

- Add public docs site generated from `docs/contracts` and module READMEs.
- Add example videos/screenshots generated from a demo workspace.

### Nice To Have

- Add changelog grouped by module.
- Add issue templates for bugs, feature requests, provider integration bugs, and security reports.

## Priority Roadmap

### P0: Must Fix Before Next Feature Expansion

1. Fix `cargo test --workspace` doctest failure.
2. Add root `AGENTS.md` and optional `CLAUDE.md` bridge.
3. Reconcile route contract drift in `docs/contracts/api.md`.
4. Harden and test the uncommitted Agent Swarm implementation.
5. Fix database Svelte warnings.
6. Add release verification script/checklist.
7. Add audit/security treatment for API tokens, network listener, destructive git/DB actions, and channel outbound replies.

### P1: Product Reliability

1. Add route inventory and RBAC matrix tests.
2. Add cross-module diagnostics/support bundle.
3. Add Product end-to-end mocked Jira/Confluence tests.
4. Add API client secret/cookie/environment hardening.
5. Add Swarm run inspector and durable coordinator recovery.
6. Add Usage/Insights provenance and parser regression corpus.

### P2: Differentiators

1. Connect Product Plan -> Swarm project/task DAG.
2. Connect Product Plan/API collections -> Workflows.
3. Add skill/context preview and eval-driven improvement loops.
4. Add review quality evals and PR merge readiness.
5. Add traceability matrix across stories, sessions, PRs, tests, decisions, and learnings.

### P3: Expansion

1. Remote/mobile access with hardened daemon-served UI.
2. Plugin/module system.
3. MCP host/server integration with tool discovery controls.
4. Team/enterprise features: SSO, managed policies, service accounts, audit exports.

## Bottom Line

Otto has enough core features to be compelling. The next meaningful improvement is making the system trustworthy under real daily use: contract sync, tests that match the wide feature surface, route/security hardening, clear diagnostics, and first-class context governance. Once that foundation is stable, the strongest product move is to connect the modules already present: Product plans should create Swarm tasks and Workflows, Swarm/Review/Product runs should feed Usage and Insights, and Skills Eval should continuously measure whether those agents are getting better.
