## Latest merge authorization (2026-10-08 local)

The user explicitly authorized admin merge after this run when only advisory tests fail: "you can have this run, if only the advisory tests fail, you can merge, again, i'm admin, so you can and than, run the review agents". Gate merge on the 17 non-advisory checks; advisory shards, advisory red-count ratchet, advisory WebKit performance, cargo audit and UI-test advisory do not block. Preserve all advisory failures as review findings and continue monitoring the running advisory jobs after merge. Do not lower thresholds, hide failures or wait for another all-advisory-green repair cycle before the authorized merge.

# Post-merge whole-repository review plan

**ACTIVE POST-MERGE REVIEW.** PR #94 merged remotely at `2026-10-07T21:42:19Z` (2026-10-08 local). Snapshot `a0bd718b9fbc008d72c164ce643a24ed78e6368c`; main parent `196048df5bbd55a691b093ea0a3fa456f0912674`, PR parent `0ea2dec98c70d2d987908df89a49ed9594fbf119`. All 17 non-advisory checks passed before the authorized admin merge. Advisory results remain reportable. Review branch `review/quality-20261008` began clean. Rust 1.99.0, Node v22.22.3 locally (CI Node 26.10.0); record that difference for reproducibility. No prior score is carried forward.

The inventory below is reconciled to the merged snapshot: add `workflow_node_driver.rs` to R08 server ownership (26 files; server total 187).

**Budget:** 16 review assignments total: 14 domain reviewers plus 2 independent integration reviewers. At most 3 workers concurrently, plus the coordinator. Reuse these workers for clarification/rechecks; do not spawn replacement review agents beyond the 16 assignments without adjusting the plan, and never exceed 20 reviewers overall. No sub-delegation. Reviewers produce evidence and proposed fixes; the coordinator owns any later authorized edits and verification scheduling.

## Review contract and execution

Every domain assignment reviews its entire owned surface, not just PR changed files, across **correctness, performance, design and UX**. Read the applicable correctness/performance/test-review skills and design guidelines after merge; UI reviews examine real rendered states. The primary lens below identifies emphasis, not an exemption from the other lenses. Non-UI crates inspect their API/CLI errors, contracts and operational usability instead of inventing visual requirements.

Each report must include: exact snapshot; inspected files/modules and seams; findings with severity, precise source lines, reachable trigger, consequence and concrete correction; traced or reproduced evidence level; mutation-checked test coverage; measured performance evidence with environment/dataset; UI screenshots where relevant; and explicit unreviewed or unverified items. No scores inferred from test counts. Distinguish existing defects, merge regressions and concurrent unrelated edits.

Review loading/empty/error/retry/loaded states, dirty drafts and cancellation, ownership across workspace/session/connection changes, focus/keyboard/RTL/phone/tablet, light/dark, long-content popup bounds, disposal and background activity. Performance checks use realistic low/high cardinalities and hot/cold paths; no broad benchmark claims from one fast sample. Tests must exercise production behavior and meaningful negative paths rather than their mocks.

**Verification rules after merge:** use only selected named Playwright specs, never the full local suite. Existing families below are a menu: select cases justified by findings and integration risk, and report the actual command/count/skips. Run relevant Rust package tests plus affected consumers, with server integration filters through `--test it`; migrations/API changes need their dedicated contract checks. One coordinator schedules Cargo commands in the worktree; do not share targets across worktrees. Keep only one expensive browser/performance/native run active at a time while other reviewers read code. Use isolated daemon slots, per-run output/auth/ports and `OTTO_E2E_SWEEP_ORPHANS=0`; fixture-owned sessions/DB objects only. No real daemon/user data, publishing, deployment or global session shutdown. Native desktop verification uses its separate workspace and an isolated app/profile.

## Assignments

### R01 — Foundation, persistence, identity and access

- Crates: `otto-core`, `otto-state` (all migrations and repositories), `otto-rbac`, `otto-keychain`, `otto-netguard`, `otto-sandbox`, `ottod` daemon startup/shutdown. `ottod` MCP bridge portions are R10's seam.
- UI: `settings`, `share`; shared auth/access stores and API adapters. Server: `boot/`, `state_archive/`, auth/policy/guards/shutdown/state, backup and access/account/settings/workspace routes (exact manifest below).
- Emphasis: rollback-compatible persistence, archive/import integrity, credential lifetime, workspace/user boundaries, configuration recovery, startup and lock contention. Access setup, denied actions and recovery must remain understandable and keyboard reachable.
- Existing verification: `otto-state/tests/migration_compat.rs`, `share_tokens.rs`, `network_profiles.rs`; server `auth_security`, `rbac_matrix`, `policy_coverage`, `share_api`, `share_scope_guard`, `share_otp`, `impersonation`, `grants_api`, `admin_sessions`, `email_sender_storage`; `desktop-backup-archive`, `desktop-backup-git`, `desktop-resource-access`, `desktop-secrets-store`, `desktop-platform-settings-recovery`, `desktop-ux-r4-access`.

### R02 — Sessions, PTY, transcripts and agent execution

- Crates: `otto-sessions`, `otto-pty`, `otto-transcript`, `otto-agent-run`, `otto-orchestrator`, `otto-context`. UI: `agents`, `panels`; terminal/conversation shared components and stores. Server: session/transcript/context/provider/liveness/event transport domains.
- Emphasis: lifecycle and epochs, reconnect/attach/kill/credential ordering, result-file recovery, source attribution, input/handover safety, terminal selection and queue behavior, bounded transcript parsing and fanout. Review session header overflow and command discovery.
- Existing verification: package lifecycle/shutdown tests; server `resume_missing_cwd`, `provider_resolve`, `session_screen`, `runtime_lag`; `desktop-agents-retention`, `desktop-view-only-attach`, `desktop-agent-handover`, `desktop-session-header`, `desktop-review4-session-recovery`, `desktop-transcript-cache-live`, `desktop-conversation-load-perf`, `desktop-terminal-rendered-load-perf`, `desktop-terminal-flood-perf`, `desktop-transport-perf`, `sessions-mobile`, `session-isolation`.

### R03 — Git, PRs, code review and issue integration

- Crates: `otto-git`, `otto-review`, `otto-issues`. UI: `git`; shared diff viewers and git/review stores. Server: repository directory/rules, findings and review helpers.
- Emphasis: staging/merge/recovery preserves work, provider/PR contract mapping, review cancellation/result identity, repo account selection, graph/diff scaling, pagination and active-tab polling, publish confirmations.
- Existing verification: package tests; server `git_spawn_guard`, `review_agent_retry`, `review_comment_states`; `git-auto-fetch-dedup`, `desktop-git-autofetch-scope`, `desktop-git-hunk-staging`, `desktop-git-conflict-resolver`, `desktop-git-recovery`, `desktop-git-pr-comment-side`, `desktop-git-pr-merge-modal`, `desktop-git-graph-perf`, `desktop-diffviewer-huge-perf`, `review-cancel`, `review-findings`, `git-mobile`.

### R04 — Connections, database engines, schemas and workbench

- Crates: `otto-connections`, `otto-dbviewer`, `otto-ssh`. UI: `connections`, `database`; database stores/editors/grids and connection adapters. Server: DB assistant/drafter/changes and connection export.
- Emphasis: connection identity, engine-specific semantics, SSH/tunnels, enforceable read-only/cancellation, schema/completion authorization, atomic edits, multi-result/tab ownership, virtual-grid/JSON scaling and bounded caches. Global library versus workspace selection must be explicit.
- Existing verification: `otto-dbviewer/tests/{mcp_query,resource_access,mysql_e2e,mongodb_e2e,postgres_e2e,redis_e2e,clickhouse_e2e}.rs`; `connections-mobile`, `connections-mcp`, `desktop-connections-reliability`, `desktop-db-comparison`, `desktop-db-json-edit`, `desktop-db-mongo-completion`, `desktop-db-mongo-keyset`, `desktop-db-shortcuts`, `desktop-db-results-perf`, `desktop-db-scale-perf`; exact `db-sweep-*` engine specs selected by impacted engine; `db-import`, `db-export`.

### R05 — API client, scripts, streaming and automation requests

- Crate: `otto-apiclient`. UI: `api`; API-client stores, editors, environment and response components. Server: API client/OAuth/cache/stream/automation and gRPC routes, API secrets/helpers.
- Emphasis: environment/request identity through async work, credentials and script sandbox boundaries, cancellation and stream disposal, history pagination/cache limits, dirty draft navigation and large-response usability.
- Existing verification: package tests; `desktop-api-run-recovery`, `desktop-api-automation-leave`, `desktop-api-environment-leave`, `desktop-api-storage`, `desktop-api-script-worker`, `desktop-api-postman-import`, `desktop-api-tabs-persist`, `desktop-review6-api-environment`, `desktop-api-scale-perf`, `desktop-api-history-performance`.

### R06 — Cloud, Kubernetes and brokers

- Crates: `otto-aws`, `otto-k8s`, `otto-brokers`. UI: `aws`, `kubernetes`, `brokers`. Server: Kubernetes monitoring scheduler; R04 owns shared SSH transport internals and supplies tunnel seam evidence.
- Emphasis: account/region/cluster scope, command cancellation, pagination/stream bounds, monitor budget/backfill, real source state after outward mutations, console loading and selection retention, required confirmation details.
- Existing verification: package tests; server `k8s_monitor_clickhouse`, `k8s_backfill_budget`; `desktop-aws-localstack`, `desktop-aws`, `desktop-kubernetes`, `desktop-k8s-fleet`, `desktop-k8s-monitor`, `desktop-k8s-resume`, `desktop-infra-perf`, `desktop-ux-r4-cloud`, `brokers-sweep`, `brokers-mobile`.

### R07 — Browser, reader/live modes and agent interaction

- Crate: `otto-browser`. UI: `browser`; browser stores/adapters. Server: browser routes/live transport/login throttle.
- Emphasis: untrusted-content boundaries, navigation/login ownership, source-to-agent fencing, DOM annotations/selectors, history/frame disposal and memory growth, readable errors and live/reader transitions.
- Existing verification: package tests; `desktop-browser-reader`, `desktop-browser-agent`, `desktop-browser-overlay-selector`, `desktop-browser-remote-live`, `desktop-ux-r4-content` browser-relevant cases. Inspect actual route/unit coverage for failure and stale-navigation branches before adding tests.

### R08 — Workflows, scheduled tasks, goal loops, runs and proof

- Crates: `otto-workflows`, `otto-automation`, `otto-workgraph`. UI: `workflows`, `scheduled-tasks`, `loops`, `run-with-otto`, `mission-control`, `proof`. Server: workflow/run engines, schedulers/contexts/channels and proof/workgraph domains.
- Emphasis: persisted state machines, restart/retry/idempotency, bounded concurrency, cancellation, stage/result ownership, proof integrity, long histories and truthful progress; dirty configuration/verification forms and approval before publication.
- Existing verification: package tests; server `run_repo_scope`, `unit/scheduled_tasks_engine`; `desktop-workflow-recovery`, `desktop-workflow-progress-auth`, `desktop-workflow-context`, `desktop-workflow-subagents`, `desktop-workflow-runview`, `desktop-review5-scheduled-draft`, `desktop-goal-draft-ownership`, `desktop-goal-loop-verification`, `desktop-run-with-otto`, `desktop-proof-packs-v2`, `mission-control`, `desktop-ux-r4-automation`, `scripts/test-scheduled-restart.mjs`.

### R09 — Assistant, personal agents, swarm, improvement and insights

- Crates: `otto-assistant`, `otto-swarm`, `otto-improve`, `otto-insights`. UI: `assistant`, `personal-agents`, `swarm`, `insights`. Server: hosts, autonomy/policy/activity and swarm ingest/webhooks, improvement delivery.
- Emphasis: identity/ownership across runs, scheduling and delegation limits, stale result protection, guardrails and persisted drafts, activity/feed scaling, notification consent and retry. R10 owns channel transport; R14 owns usage metrics sourcing.
- Existing verification: package tests; server `personal_agent_policy`, `swarm_scope_api`, `activity_isolation`; `desktop-assistant`, `desktop-personal-autonomy`, `desktop-personal-documents`, `desktop-review5-personal-agent-ownership`, `desktop-swarm-session-panel`, `desktop-swarm-empty-workspace`, `swarm-goals`, `swarm-mobile`, `desktop-insights`, `desktop-performance-agents-workflows`, `desktop-ux-r4-insights`.

### R10 — MCP governance, UI control, channels and live rooms

- Crates: `otto-mcp`, `otto-channels`; `ottod` inward/outward MCP entrypoints with R01/R02. UI: `mcp`, `rooms`; all `lib/uiCommands`. Server: `mcp_*`, UI bridge/commands, `rooms/`, channel webhooks.
- Emphasis: enabled catalog/session-grant consistency, approvals and revocation, UI device ownership/headless behavior, token isolation, room consent/media/archive identity, WebSocket lifecycle/backpressure, retries without duplicate outbound effects. Explicitly inspect commands across DB/API/git/product seams with owners.
- Existing verification: package tests; server `ui_control`, `mcp_auto_approve`, `rooms_api`; `desktop-agent-ui-control`, `desktop-mcp-api-tools`, `desktop-mcp-cp-tokens`, `desktop-review5-mcp-audit-details`, `mcp-http-tokens`, `mcp-otto-features`, `desktop-rooms-live`, `desktop-room-window`, `room-recap`, `desktop-channels-rooms-health`.

### R11 — Vault, memory, indexing and documentation runs

- Crates: `otto-vault`, `otto-memory`. UI: `vault`; Vault/markdown stores and rendering. Server: memory governance, product memory and Vault docs agent.
- Emphasis: file/index consistency, path safety, OKF contract completeness, backlink and graph scaling, watcher/rebuild/retry ownership, bounded search/context and evidence precision, editing/draft recovery on phone and tablet.
- Existing verification: package tests; `desktop-vault-docs`, `desktop-vault-agent-runs`, `desktop-vault-agents`, `desktop-vault-recovery`, `desktop-review5-vault-backlinks`, `desktop-vault-structured`, `desktop-vault-graph-filter`, `desktop-vault-performance`, `desktop-vault-scale-perf`, `vault-mobile`.

### R12 — Product, Canvas and Design Hall

- Crates: `otto-product`, `otto-canvas`, `otto-design`, `otto-design-assist`. UI: `product`, `canvas`, `design-hall`; shared SVG/markdown/code rendering seams with R11/R13. Server: product host, mockup/design/scene3D and canvas-reference domains.
- Emphasis: artifact/version identity, autosave and concurrent edits, assist cancellation/recovery, publication and imported untrusted assets, queued renderer disposal/bfcache, 3D assets and editor scaling, responsive source/preview and recoverable failures.
- Existing verification: package tests; server `canvas_refs_api`; `desktop-review4-product-publication`, `desktop-review6-product-routing`, `product-attachments`, `product-refine`, `product-design-arena`, `desktop-review5-canvas-assist-recovery`, `desktop-canvas-versions`, `desktop-design-hall`, `desktop-design-cocreate`, `desktop-studio-3d`, `desktop-ux-r4-content`, `desktop-canvas-scale-perf`, `desktop-design-hall-scale-perf`.

### R13 — Native desktop, shared shell, home/school and workbench

- Separate workspace: `apps/desktop/src-tauri` source, capabilities, examples/probes and packaging integration; `apps/desktop` remaining source. UI: `desktop`, `home`, `workbench`, `help`, `snip`; `App.svelte`, `shell/`, top-level lib utilities/tokens/sidebar, shared components/stores not assigned to feature owners, walkthroughs. Server: LSP, workbench/history/filesystem/snips, navigation/meta/notifications and CLI update.
- Emphasis: native/browser parity, multiwindow/pane lifecycle, key routing/focus, scroll/resize/zoom, file access, workbench persistence, school resource disposal and movement, shared-chrome consistency, background work and frame budgets.
- Existing verification: desktop separate-workspace checks/probes; server `workbench_api`, `snips`, `router_mount`; `desktop-multiwindow`, `desktop-detachable-panes`, `desktop-session-header`, `desktop-page-header-more-menu`, `desktop-workbench`, `desktop-home-classrooms-3d`, `desktop-home-classrooms-regressions`, `desktop-home-classrooms`, `desktop-review4-workbench-history`, `desktop-modal-focus-resize`, `desktop-shell-perf`, `desktop-nav-smooth-perf`, `desktop-boot-perf`, `nav-mobile`, `rtl`, `theme` (named selections only).

### R14 — Plugins, skills/evaluation, usage and telemetry

- Crates: `otto-skills`, `otto-usage`, `otto-telemetry`. Both runtime plugins: `examples/plugins/team-performance`, `examples/plugins/dora-metrics` (separate Cargo crate). UI: `plugins`, `skills-eval`, `skills-lab`, `usage`; telemetry/settings seam with R01. Server: plugin supervisor, evaluation/review/scoring, usage and telemetry.
- Emphasis: plugin lifecycle/protocol and isolation, schema/version compatibility, bounded analytics retention/queries, usage dedup/cache accounting, metric/window correctness and attribution, evaluation ownership and truthful results, legibility of charts/tables and export/report flows.
- Existing verification: package tests; `desktop-plugins`, `desktop-skills-lab`, `desktop-eval-lab`, `desktop-usage`, `desktop-telemetry`, `desktop-telemetry-matched-perf`; team-performance `test/{metrics,analytics,dora,scope-rules,phases,reportmodel,cancellation,server.e2e,browser.e2e}.test.js`; DORA Cargo tests. Run plugin suites serially through the documented gate.

### R15 — Independent integration: correctness, performance and test quality

- Start only after R01–R14 reports. Own root build/config/CI/scripts/tooling, `dev/`, `vendor/` integration patches, unassigned server composition glue, shared API contract parity and verification harness. Audit all test infrastructure, fixtures, named gate selection, skip/failure ratchets and global mutable state. Domain-specific test files stay with their owners; this reviewer independently challenges their oracles and coverage.
- Trace cross-domain flows: session -> credential -> MCP -> UI result; repository -> review -> findings -> run/proof; product -> workflow/swarm -> publication; DB connection -> access policy -> query/edit -> reconnect; rooms -> consent -> archive; deployment/restart -> compatible state and credential recovery. Review startup/shutdown/task ownership and shared state contention across all domains.
- Existing verification: server `route_inventory`, `policy_coverage`, `router_mount`, `runtime_lag`, `git_spawn_guard`, `run_repo_scope`; migration compatibility; root scoped/full CI-equivalent Rust gate as justified; UI check/unit/build/bundle budget; `scripts/test_loc_ratchet.py`, `scripts/test_check_nextest_filters.py`, `scripts/test_macos_ci_crates.py`; fixture isolation via selected colliding spec pairs. Verify the merged CI SHA and zero-failure ratchet rather than relying on check colour alone.
- Output: source-supported integration findings, challenged/falsified domain claims, dependency impact checklist, exact remaining validation gaps. No automatic approval from green tests.

### R16 — Independent integration: product design and UX

- Start only after R01–R14. Read all UI module reports and shared-shell/native evidence. Own `docs/design/`, user-facing help/walkthrough/feature-guide consistency, `marketing/` and user-facing packaging copy; sample every sidebar module with its true loaded/empty/error state and trace cross-module journeys. Validate earlier reviewer screenshots against the final merged build.
- Cross-domain journeys: setup/login/workspace -> session -> Git/PR; agent -> side pane -> DB/API result -> stop control; product -> design/mockup -> publication; workflow/scheduled task -> progress/error -> recovery; school -> session; connection -> query -> compare/edit -> leave; rooms -> share/consent -> archive. Focus accessible structure, target identity, durable drafts, clear errors/actions, visual hierarchy, responsive/RTL behavior and native shortcuts.
- Existing verification: `desktop-page-chrome`, `desktop-dialogs-tokens`, `desktop-load-errors`, `desktop-journeys`, `desktop-ux-audit-sweep` and explicitly selected `desktop-ux-r4-*`, `desktop-pane-header-overflow`, `desktop-modal-focus-resize`, `desktop-help`, `desktop-ux-r4-help`, `nav-mobile`, `theme`, `rtl`; native probes from R13. Do not substitute broad screenshots for successful task completion.
- Output: evidence-backed cross-module design/UX findings, unresolved design-state matrix and contradictions between UI, contracts and guides. No numeric quality target presumed met.

## Scheduling and completion

Use successive worker waves: (R01,R02,R03), (R04,R05,R06), (R07,R08,R09), (R10,R11,R12), (R13,R14), then (R15,R16). Reorder domain waves only for resource availability; integration reviewers still wait for all domain reports. Carry seam questions between existing reviewers rather than spawning new ones. The coordinator grants a single heavy verification lease and records runs against the pinned merge SHA.

Before each wave, expand the owned paths below to a file manifest and attach it to the assignment. Every root crate and UI module has one primary owner; shared seams name the second reviewer. Every server Rust file must receive an owner (manifest below); unexpected/new paths are explicitly assigned before review starts. R15 owns the repository-wide contract/test/build integration inventory, R16 owns consistency across all user surfaces. Generated outputs (`target`, `node_modules`, `dist`, generated icons/schema bundles) are not line-reviewed as source; their generators/build/load paths are assigned. Do not silently omit tracked source inside vendor/tools/packaging.

After findings are resolved, re-review changed seams with the same reviewers, run only necessary regression/consumer checks, and obtain R15/R16's independent final verdicts. A report may conclude not ready. Final delivery distinguishes inspected coverage, measured results, real test execution, skipped/unavailable environments and remaining risk; it must not claim exhaustive runtime proof or a 9.8 score from completion of this plan.

## Exhaustive ownership inventories

All paths below were inventoried read-only before merge; reconcile additions/removals at the actual merge SHA.

Inventory reconciliation: **43 root crates, 37 UI module directories, 187 server Rust source files; zero unassigned.** `otto-server` is partitioned across domain owners below; R15 owns integration glue.

| Assignment | Root crates | UI modules |
| --- | --- | --- |
| R01 | `otto-core`, `otto-keychain`, `otto-netguard`, `otto-rbac`, `otto-sandbox`, `otto-state`, `ottod` | `settings`, `share` |
| R02 | `otto-agent-run`, `otto-context`, `otto-orchestrator`, `otto-pty`, `otto-sessions`, `otto-transcript` | `agents`, `panels` |
| R03 | `otto-git`, `otto-issues`, `otto-review` | `git` |
| R04 | `otto-connections`, `otto-dbviewer`, `otto-ssh` | `connections`, `database` |
| R05 | `otto-apiclient` | `api` |
| R06 | `otto-aws`, `otto-brokers`, `otto-k8s` | `aws`, `brokers`, `kubernetes` |
| R07 | `otto-browser` | `browser` |
| R08 | `otto-automation`, `otto-workflows`, `otto-workgraph` | `loops`, `mission-control`, `proof`, `run-with-otto`, `scheduled-tasks`, `workflows` |
| R09 | `otto-assistant`, `otto-improve`, `otto-insights`, `otto-swarm` | `assistant`, `insights`, `personal-agents`, `swarm` |
| R10 | `otto-channels`, `otto-mcp` | `mcp`, `rooms` |
| R11 | `otto-memory`, `otto-vault` | `vault` |
| R12 | `otto-canvas`, `otto-design`, `otto-design-assist`, `otto-product` | `canvas`, `design-hall`, `product` |
| R13 | — | `desktop`, `help`, `home`, `snip`, `workbench` |
| R14 | `otto-skills`, `otto-telemetry`, `otto-usage` | `plugins`, `skills-eval`, `skills-lab`, `usage` |
| R15 | `otto-server` | — |

Server file manifest (relative to `crates/otto-server/src/`; complete owned files, including nested test modules):

- **R01** (38 files): `auth.rs`, `boot/build.rs`, `boot/ctx.rs`, `boot/mod.rs`, `boot/open.rs`, `boot/recovery.rs`, `boot/tasks.rs`, `feature_guard.rs`, `host_guard.rs`, `login_throttle.rs`, `policy.rs`, `routes/access_groups.rs`, `routes/admin_sessions.rs`, `routes/audit.rs`, `routes/auth_routes.rs`, `routes/backup.rs`, `routes/backup_git/files.rs`, `routes/backup_git.rs`, `routes/capabilities.rs`, `routes/email_sender.rs`, `routes/grants.rs`, `routes/impersonate.rs`, `routes/network_profiles.rs`, `routes/onboarding.rs`, `routes/projects.rs`, `routes/provider_accounts.rs`, `routes/resource_access.rs`, `routes/resource_access_tests.rs`, `routes/settings.rs`, `routes/share.rs`, `routes/users.rs`, `routes/workspaces.rs`, `shutdown.rs`, `state.rs`, `state_archive/files.rs`, `state_archive/schema.rs`, `state_archive/tests.rs`, `state_archive.rs`.
- **R02** (19 files): `agent_refs.rs`, `agent_session.rs`, `agent_tasks_nudge.rs`, `context_packet.rs`, `history_index.rs`, `live_events/tests/mod.rs`, `live_events.rs`, `model_catalog.rs`, `provider_resolve.rs`, `resource_sessions.rs`, `routes/handover.rs`, `routes/name_themes.rs`, `routes/slash_commands.rs`, `routes/transcript.rs`, `transcript_cache.rs`, `transcript_tail.rs`, `transport.rs`, `ws_events.rs`, `ws_fanout.rs`.
- **R03** (5 files): `finding_agent.rs`, `finding_context.rs`, `repo_directory.rs`, `routes/findings.rs`, `routes/repo_rules.rs`.
- **R04** (5 files): `database_changes.rs`, `db_assist.rs`, `db_drafter.rs`, `routes/connection_export.rs`, `routes/database_changes.rs`.
- **R05** (8 files): `api_helpers.rs`, `api_secrets.rs`, `routes/api_automation_runs.rs`, `routes/api_client.rs`, `routes/api_oauth.rs`, `routes/api_response_cache.rs`, `routes/api_stream.rs`, `routes/grpc.rs`.
- **R06** (1 files): `k8s_monitor_scheduler.rs`.
- **R07** (3 files): `browser_login_throttle.rs`, `routes/browser.rs`, `routes/browser_live.rs`.
- **R08** (26 files): `automation_ctx.rs`, `proof.rs`, `routes/goal_loops.rs`, `routes/mission.rs`, `routes/proof.rs`, `routes/proof_pack.rs`, `routes/runs.rs`, `routes/scheduled_tasks.rs`, `routes/workflow_progress.rs`, `routes/workflows.rs`, `routes/workgraph.rs`, `run_callback.rs`, `run_channels.rs`, `run_context.rs`, `run_engine.rs`, `run_notices.rs`, `run_scheduler.rs`, `run_service.rs`, `run_sources.rs`, `run_workspace.rs`, `workflow_engine.rs`, `workflow_node_driver.rs`, `workflow_product_publish.rs`, `workflow_product_publish_tests.rs`, `workflow_trigger_scheduler.rs`, `workgraph_projector.rs`.
- **R09** (10 files): `assistant_host.rs`, `improve_channels.rs`, `personal_agent_activity.rs`, `personal_agent_policy.rs`, `routes/assistant.rs`, `routes/personal_agents/autonomy.rs`, `routes/personal_agents.rs`, `routes/swarm_ingest.rs`, `routes/swarm_webhook.rs`, `swarm_host.rs`.
- **R10** (31 files): `mcp_auto_approve.rs`, `mcp_capabilities.rs`, `mcp_http.rs`, `mcp_outward.rs`, `rooms/actions.rs`, `rooms/annotations.rs`, `rooms/auth.rs`, `rooms/http.rs`, `rooms/ice.rs`, `rooms/mod.rs`, `rooms/presentation.rs`, `rooms/recap/archive.rs`, `rooms/recap/consent.rs`, `rooms/recap/http.rs`, `rooms/recap/media.rs`, `rooms/recap/mod.rs`, `rooms/recap_engines/chunks.rs`, `rooms/recap_engines/mod.rs`, `rooms/recap_engines/process.rs`, `rooms/recap_engines/speech.rs`, `rooms/recap_engines/summary.rs`, `rooms/recap_engines/vision.rs`, `rooms/registry.rs`, `rooms/socket.rs`, `rooms/terminal.rs`, `rooms/tests.rs`, `routes/channel_webhook.rs`, `routes/mcp_cp.rs`, `routes/mcp_servers.rs`, `routes/ui_commands.rs`, `ui_bridge.rs`.
- **R11** (3 files): `memory_gov.rs`, `routes/product_memory.rs`, `vault_docs_agent.rs`.
- **R12** (7 files): `canvas_refs.rs`, `design_blender.rs`, `design_format.rs`, `design_hall.rs`, `design_scene3d.rs`, `mockup_assist.rs`, `product_host.rs`.
- **R13** (13 files): `cli_update.rs`, `lsp/framing.rs`, `lsp/mod.rs`, `lsp/pool.rs`, `lsp/servers.rs`, `routes/fs.rs`, `routes/history_page.rs`, `routes/logs.rs`, `routes/meta.rs`, `routes/notifications.rs`, `routes/search.rs`, `routes/snips.rs`, `routes/workbench.rs`.
- **R14** (9 files): `eval_lab_routes.rs`, `eval_score.rs`, `plugins.rs`, `routes/telemetry.rs`, `routes/usage.rs`, `skill_eval.rs`, `skill_review.rs`, `skill_review_static.rs`, `telemetry.rs`.
- **R15** (9 files): `error.rs`, `lib.rs`, `modules.rs`, `monitor.rs`, `routes/activity.rs`, `routes/mod.rs`, `self_call.rs`, `spa.rs`, `test_support.rs`.

## Assessment rubric (agreed internally before review, not a result)

Scores are bounded engineering assessments out of 10, separately from evidence confidence. A numeric target never authorizes suppressing findings, weakening tests, raising budgets, or marking an untested surface verified. Domain and integration reviewers must list the concrete evidence that prevents or supports 9.8; a repository-wide score cannot exceed a material domain gap without explaining and resolving it.

- Correctness: reachable invariants, state transitions, isolation, persistence/recovery, cancellation, negative paths and contract compatibility. Outstanding critical/high defects preclude 9.8; material medium defects also require closure. Green CI alone is insufficient.
- Performance: bounded work/memory/concurrency, realistic cardinality, expensive operations measured on representative workloads, no known unbounded hot path, and explicit device/renderer/network/environment limits. Functional durations do not establish performance quality. Unmeasured central workloads remain an evidence gap.
- Design: coherent module responsibilities and dependency direction plus consistent information hierarchy, tokens, shared components, state coverage and responsive layout. Taste-only redesigns are not findings. Outstanding structural or visual defects that impede expected change/use require closure.
- UX/usability: successful primary and recovery journeys, discoverable actions, honest feedback, safe target identity, preserved drafts, accessible keyboard/focus and readable error/loading/empty states. Source inspection cannot certify native or assistive-technology behavior not exercised.

Use 9.8 only when the inspected scope has no unresolved material findings and the listed critical journeys/workloads have appropriate evidence; retain the final 0.2 as residual uncertainty, not a guarantee. If the evidence does not support the requested target, continue the repair/verification cycle or explicitly identify the external dependency preventing completion. Report exact inspected scope and confidence alongside any final numbers.
