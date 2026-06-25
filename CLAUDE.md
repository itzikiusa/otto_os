# CLAUDE.md

This repository's guidance for AI agents lives in **[AGENTS.md](./AGENTS.md)** —
build/test commands, the crate & UI module map, conventions, and the rules for
not damaging user work. Read it before making changes.

@AGENTS.md

<!-- OTTO:START -->
## Memory

# Memory Index

- [Otto Goal Loops](otto-goal-loops.md) — bounded multi-agent iteration toward a goal (Plan→Execute→Evaluate→Digest, isolated worktree, machine-checked stop); built+verified, uncommitted
- [Otto webhook channel](otto-webhook-channel.md) — inbound HTTP webhook channel (POST /api/v1/webhooks/{ws}, key-auth, reuses Bridge); near-zero schema, own Bridge instance; verified green, uncommitted
- [Otto swarm board + configurator](otto-swarm-board-and-configurator.md) — agent-configurator IS the existing Recruiter (don't duplicate); fixed dead Plan-from-goal (pid/select desync), added goal viewer/editor + reconnect-now; deployed, uncommitted

- [Otto ADE project](otto-ade-project.md) — Tauri/Rust/Svelte rewrite of loom; no commits until user says; self-signed "Otto Dev Signing"; channels/skills/swarm deferred
- [Otto mobile E2E + responsive](otto-mobile-e2e-responsive.md) — Playwright harness (isolated test daemon, 5 iPhone/iPad profiles), .mcenter min-height fix + tablist a11y fixes; 240/240 green but EMPTY-state only; drill-down+tabs redesign + seeded specs are next; uncommitted
- [Otto parallel-batch workflow](otto-workflow-parallel-batches.md) — user rapid-fires independent requests; fan out parallel agents, lock shared contracts first, verify don't commit
- [Otto ottod deploy](otto-ottod-deploy.md) — ottod runs under launchd; cargo build alone won't update the running daemon — rebuild release + swap installed/bundle + launchctl kickstart
- [Otto deploy: do it myself](otto-deploy-do-it-myself.md) — user must NEVER restart/replace the app; Claude runs the whole build→sign→install→force-quit→relaunch→activate cycle and leaves it running on the new build
- [Otto self-improvement & context](otto-self-improvement-and-context.md) — 2026-06-13 (uncommitted): crates otto-improve (scheduled self-reflection + version log) & otto-context (skills/souls/soul + per-CLI materialization via PreSpawnHook); library is source-of-truth; live evolver deferred
- [Otto review-sessions + robustness status](otto-review-sessions-status.md) — DONE+DEPLOYED: default-agent system, PromptGuard anti-stuck, review-agents-as-live-sessions (Open/findings/waiting). Otto.app rebuilt+deployed — user must quit+reopen for new UI. Follow-ups: UI-configurable timeout, validate codex/agy prompt patterns
- [Otto Slack relay attachments](otto-slack-relay-attachments.md) — accept-ALL inbound (no subtype drop; download files to tmp), ⟦otto-file⟧ outbound directive, loop-prevention via nested bot_id; the review "cap" was a lost-update race fixed via set_agent_at/json_replace
- [Otto channel session retention](otto-channel-session-retention.md) — ticketing volume tuning: archive idle channel sessions after 1h, sidebar cap 20 most-recent + collapsed-by-default, delete archived channel sessions after 30 days
- [Otto connections are global](otto-connections-global.md) — connections = global library (not workspace-scoped); sessions attach to a workspace temporarily per-session (explicit workspace picker on open); folders = one shared global tree; never-hide rule (unknown folder → Ungrouped)
- [Otto DB Explorer Mongo](otto-db-explorer-mongo.md) — active-DB `node` is a plain name (all drivers); SQL→Mongo translator in mongo_sql.rs (SELECT triggers it, generated query shown in result banner); Find Rows menu; visual builder deferred; Redis large-keyspace prefix filter + bounded SCAN + pipelined TYPE + truncation hint
- [Otto webview zoom crispness](otto-webview-zoom-crispness.md) — terminal blur fix: use native WKWebView page-zoom (setZoom) not CSS `zoom`; CSS zoom stretches the WebGL canvas → soft text; never wrap terminal in zoom/transform/filter
- [Otto selection contrast](otto-selection-contrast.md) — selection/active highlights must be high-contrast light-green (#7ee787) + black, NOT a % of the dark-blue --accent; test on dark scheme
- [Otto Svelte5 derived mutation](otto-svelte5-derived-mutation.md) — store getter that lazily mutates $state, read inside $derived → state_unsafe_mutation → silent blank render (caused SFTP "Browse files" no-op); fix = pure read + separate ensure()
- [Otto terminal multi-socket dup](otto-terminal-multisocket-dup.md) — PTY output is broadcast to all /ws/term clients; a Terminal must hold exactly ONE socket or output (incl. keystroke echo) duplicates. connect() must close the prior socket — reconnect+activate race (deploy) leaked 2 sockets → "each keystroke duplicated"
- [Otto webview getter panic aborts](otto-webview-getter-panic-aborts.md) — calling wry getters (webview.url()) on a not-yet-loaded child webview panics, poisons the shared window_id mutex, and ABORTS across WebKit's extern-C boundary; catch_unwind insufficient (poison) — track URL via on_navigation, never poll url(). Bit the per-tab browser; native-zoom child bounds = rect × ui.zoom
<!-- OTTO:END -->