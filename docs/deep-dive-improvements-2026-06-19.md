# Otto — Deep-Dive Audit & Improvement Roadmap

> **Date:** 2026-06-19
> **Scope:** Whole repository — ~20 Rust crates (~70K LOC) + Svelte 5 UI (~130 components), across every feature section.
> **Method:** Eight parallel deep-dive audits, each owning a feature cluster, reading the backend + UI + contracts/specs and flagging real bugs/races separately from enhancements. Read-only — **no code was changed.**
> **How to read this:** Each section lists *Current state → Strengths → Gaps/Issues → Recommendations*, and every recommendation set is ordered **Must-have → Should-have → Nice-to-have**. The first section is an app-wide rollup of the highest-priority items that cut across features.

---

## 0. App-wide top priorities (read this first)

The single most important theme: **Otto is hardened for `127.0.0.1`-only use, but the remote/mobile goal (`network_listener` on `0.0.0.0` + Tailscale) removes that boundary.** Several endpoints are safe *only* because they're loopback-bound today. Fix the security items below **before** anyone exposes the daemon to a network.

### Security-critical (fix before any remote/Tailscale exposure)

| # | Issue | Where | Section |
|---|-------|-------|---------|
| S1 | **SSRF / open proxy** — API-client `execute`/`oauth2`/gRPC/stream and `browser_proxy` fetch arbitrary user URLs with no private-IP/loopback/`169.254.169.254` guard, follow redirects, allow TLS-skip | `routes/api_client.rs:62-93,700-905`, `modules.rs` browser_proxy | Core (§2), Cross (§8) |
| S2 | **Arbitrary host-file read** — `GET /fs/browse` & `/fs/read` accept any absolute/`~` path, return contents, discard the `CurrentUser`; reads `~/.ssh/id_rsa`, `~/.aws/credentials`, `/etc/passwd` | `routes/fs.rs:106-279` | Core (§2) |
| S3 | **No TLS + token-in-URL on the network listener** — `0.0.0.0` reuses the same router over **plain HTTP**; WS bearer token travels in `?token=` (logged everywhere) | `ottod/src/main.rs:531-548`, `ui/src/lib/api/client.ts` | Core (§2) |
| S4 | **Cross-user Jira/Confluence credential leak** — every issue read/use handler takes `Extension(_user)` and never checks `account.user_id`; any authed user acts with the owner's Atlassian identity | `otto-issues/src/http.rs:205-499` | Product (§6) |
| S5 | **No login rate-limiting** — `POST /auth/login` is unthrottled → online password brute-force | `routes/auth_routes.rs:17` | Core (§2), Cross (§8) |
| S6 | **Self-improvement memory poisoning** — untrusted session/Jira text is interpolated unescaped into the improve prompt; Memory edits bypass the allow-list and auto-apply on self-reported `Low` risk → persistent `MEMORY.md` poisoning that influences every future agent | `otto-improve/src/{prompt.rs,classify.rs,engine.rs:356-373}` | Intelligence (§7) |
| S7 | **DB Explorer privilege escalation** — `run_widget` requires only `Viewer` but executes arbitrary stored SQL (incl. writes/DDL) | dbviewer `http.rs:469-479` | Database (§5) |
| S8 | **TLS client private keys world-readable in `/tmp`** — inline PEM written with default perms, never deleted | dbviewer `tls.rs:16-26` | Database (§5) |
| S9 | **Notifications are global, cross-user** — handlers ignore `CurrentUser`; every `Notification` broadcasts to all clients; any user can clear/alter global notification state | `routes/notifications.rs`, `ws_events.rs:93` | Core (§2) |
| S10 | **Tauri CSP disabled** (`"csp": null`) while the webview loads remote content via the proxy | `apps/desktop/src-tauri/tauri.conf.json:26` | Cross (§8) |
| S11 | **Symlink escape in git conflict resolve** — `safe_join` blocks `..` but not symlinks → write outside the work-tree | `otto-git/src/local.rs:756-773` | Git (§4) |

### Data-loss / correctness-critical

| # | Issue | Where | Section |
|---|-------|-------|---------|
| D1 | **Swarm worktree branch reset every turn** — `ensure_cwd` calls `worktree_add -B` unconditionally, wiping the agent's uncommitted work each turn | `swarm_workspace.rs::ensure_cwd` | Swarm (§3) |
| D2 | **Swarm backlog tasks never run** — `create_task` defaults `status:"backlog"`, but `ready_tasks` only selects `"todo"`; no transition exists | `otto-swarm/service.rs`, `otto-state/swarm.rs` | Swarm (§3) |
| D3 | **Swarm has no spend/run/time budget** — only a concurrency cap; loops (re-queue, handoff-of-handoff) are unbounded → uncapped token spend | `swarm_runtime.rs::tick/route_result` | Swarm (§3) |
| D4 | **Terminal scrollback lost on every reattach** — snapshot sends only the visible screen, never the 1000-line parser / 10k-line ring | `otto-pty/src/lib.rs:213`, `ws.rs:255` | Sessions (§4… listed as §3.5) |
| D5 | **TiledView resurrects every suspended session** — one live WS per active session, no cap → defeats idle-suspend (~200 MB/agent) | `TiledView.svelte:56`, `SessionView.svelte:294` | Sessions |
| D6 | **Usage cost is materially wrong** — cache tokens never priced; pricing table stale: Opus over-billed ~3×, Fable 5/unknown models silently $0 | `otto-usage/src/pricing.rs` | Intelligence (§7) |
| D7 | **Workflow runs have no crash recovery** — a `running` row whose task died stays "running" forever; no reaper, no timeout | `workflow_engine.rs`, `main.rs:249-266` | Core (§2) |
| D8 | **Swarm runs have no stale-run reconciliation** — post-crash, non-terminal runs permanently consume the cap and block their agent | `swarm_runtime.rs`, `main.rs:457-467` | Swarm (§3) |
| D9 | **Git merge breaks in worktrees/submodules** — `merge_source` hardcodes `.git/MERGE_MSG`; reachable via the swarm's worktrees | `otto-git/src/local.rs:669-670` | Git |
| D10 | **Untracked files silently excluded** from local review and PR-draft (uses bare `git diff <base>`) | `local.rs:335-337`, `modules.rs:1494,1857` | Git |

### Reliability / process

- **No CI/CD at all** — 471 Rust tests never gate a change; add `cargo test`/`clippy -D warnings`/`fmt --check`/`audit`.
- **No HTTP timeouts** on any Jira/Confluence/Slack/Telegram client — a hung endpoint blocks the watcher indefinitely.
- **Slack `files.upload` is deprecated/sunset** — attachment upload will break.
- **DB query cancel is client-only** — "Stop" abandons the request while the DB keeps working.
- **Zero UI tests** across 130 components; the newest features (Swarm) are near-untested.

---

## 1. Cross-Cutting Concerns (Testing, Security, Build, Docs, Observability, Performance, Deps, A11y/i18n)

### Current state
- **Testing:** 71 files with `#[cfg(test)]`, ~471 test fns (393 `#[test]`, 78 `#[tokio::test]`). Heavy in `otto-server`/`otto-dbviewer`/`otto-state`/`otto-improve`; thin-to-zero in `otto-keychain` (0), `otto-pty` (1), `otto-rbac` (1), `otto-core` (1), `otto-swarm` (1). **UI: zero tests** (no vitest/playwright, no `test` script) across 130 components. Swarm (~3,200 LOC runtime) ships ~5 tests.
- **Security primitives (good):** argon2id passwords; 32-byte `OsRng` bearer tokens stored only as SHA-256; Keychain-backed secrets (0600 file fallback for dev); auth middleware on `/api/v1`; `0.0.0.0` reuses the auth-protected router; argv-array shell-out everywhere (no `sh -c`); partial prompt-injection mitigation in the channel bridge.
- **Build/Packaging:** Tauri 2 bundling `ottod` as an `externalBin`; **self-signed only** (`packaging/sign.sh`, `--timestamp=none`), **no notarization, no auto-updater**; `"csp": null`; **no CI/CD** (`.github/workflows` and `bitbucket-pipelines.yml` absent); manual build + launchd on 7700.
- **Docs:** 19/19 lib crates have module `//!` docs; `api.md` documents 86 endpoints but code registers **149 routes** (~14 REST areas undocumented); `ws.md` documents ~5 of 16 `Event` variants; `otto-swarm` missing from README.
- **Observability:** `tracing` + daily-rolling file log; **no metrics, no crash/error reporting, no panic hook** — a daemon panic dies silently.
- **Performance:** SQLite well-tuned (WAL/FK/busy_timeout/8-pool, 30 migrations); bounded `broadcast::channel(1024)`. UI bundle: single **2.5 MB** `index-*.js`, 396 KB CSS, 31 MB `dist` (Monaco/xterm + bundled fonts); `ottod` binary 45 MB.
- **Deps:** clean centralized workspace table, modern (axum 0.8, sqlx 0.8, reqwest 0.12 rustls-only, tokio 1, argon2 0.5); 553 locked crates; benign transitive duplicates; **`cargo-audit` never run**.
- **A11y/i18n:** real but ad-hoc RTL/Hebrew; **no i18n framework** (hardcoded English); 62/130 components use `aria-*`/`role`, but **47 `svelte-ignore a11y-*` suppressions**.

### Strengths
Sound auth/crypto foundation; argv-array execution (no shell injection); broad Rust unit testing; ~100% module rustdoc; well-tuned SQLite + lag-tolerant WS; modern rustls-only deps; honest "not audited" disclaimer; prod `.unwrap()`/`.expect()` confined to tests.

### Recommendations
#### Must-have
- Add rate-limiting/backoff to `POST /auth/login` (`routes/auth_routes.rs:19`) — biggest auth gap given the remote goal.
- Add an SSRF guard (reject loopback/private/link-local/metadata) on `browser_proxy` and the API-client executor (`modules.rs`, `routes/api_client.rs:740`).
- Add a minimal CI pipeline (`cargo test` + `clippy -D warnings` + `fmt --check` + `cargo audit`) — nothing gates changes today.
- Set a real Tauri CSP instead of `"csp": null` (`apps/desktop/src-tauri/tauri.conf.json:26`).

#### Should-have
- Stand up UI test tooling (vitest + a few playwright smokes) — 130 components, 0 tests.
- Backfill tests for the Swarm runtime and `otto-keychain`/`otto-pty` (newest + security-critical, least tested).
- Add a daemon panic hook + optional opt-in crash reporting.
- Code-signing/notarization path + a Tauri auto-updater (current self-signed build needs manual Gatekeeper bypass, no patch channel).
- Sync the frozen contracts: enumerate all 16 `Event` variants in `ws.md`, document the ~14 undocumented REST areas, add `otto-swarm` to README.

#### Nice-to-have
- Code-split the 2.5 MB JS (lazy-load Monaco/xterm/dbviewer); subset the bundled fonts (`ui/dist` is 31 MB).
- Introduce a lightweight i18n layer (locale JSON + `t()` helper) so the RTL work has a translation path.
- Burn down the 47 `svelte-ignore a11y-*` suppressions; add a `/metrics` Prometheus endpoint.
- Run `cargo audit` once to baseline the 553-crate tree.

---

## 2. Core Backend, API Client, Workflows & Auth/RBAC

### Current state
`ottod` on `127.0.0.1:7700` (optional `0.0.0.0` via `network_listener`, `main.rs:509-551`) serving HTTP+WS to the Svelte SPA. `otto-server::build_router` nests module routers under `/api/v1` behind bearer-auth. Auth = a clean trait split (`otto-core::auth` traits, `otto-rbac` impl over SQLite; SHA-256 token hash, 30-day sliding, argon2id). RBAC = global `root` + per-workspace `viewer<editor<admin`. Secrets → Keychain (file fallback). State = 29 forward-only `sqlx` migrations (WAL, FK on, busy_timeout 5s, 8 conns). API client = daemon-proxied REST/gRPC/SSE/WS workbench (collections, environments, history, OAuth2, cookies). Workflows = DAG engine (agent/http/transform/delay/game nodes). Events = `broadcast(1024)` over `/ws/events` with per-workspace role filtering.

### Strengths
Clean core-trait/rbac split; hash-only tokens with UNIQUE index + `ON DELETE CASCADE`; consistent `Error → Problem{code,message}` mapping; WS auth validated before upgrade + role-gated; per-session ingest tokens; graceful shutdown drains both listeners and kills all PTYs; orphan-recovery for reviews/skill-evals/product/swarms; documented "FROZEN" v1 with `api_version`; API client correctly proxies through the daemon.

### Gaps / Issues found
- **SSRF (CRITICAL)** — see S1. `routes/api_client.rs:62-93,700-905`, `api_stream.rs`.
- **`fs.rs` arbitrary file read (CRITICAL for remote)** — see S2. `routes/fs.rs:106-279`.
- **Network listener weakens the boundary** — plain HTTP, token in `?token=` query (`main.rs:540`). Tailscale only encrypts on-tailnet hops.
- **Workflow runs not crash-recoverable** — no startup reaper, no global run timeout, unbounded concurrent runs (`workflow_engine.rs:72-219`, `routes/workflows.rs:302`).
- **WS message loss on reconnect** — `Lagged` drops with a warn (`ws_events.rs:65`); UI `onopen` triggers no resync (`events.svelte.ts:43-45`); no sequence numbers/replay.
- **Notifications global, not per-user** — cross-user leak + global mutation (`routes/notifications.rs`).
- **Token lifecycle gaps** — password change doesn't revoke `auth_sessions` (`routes/users.rs:58-71`); no "revoke all"; no login rate-limit; non-constant-time token-hash compare (low risk).
- **No password policy outside onboarding** (`routes/users.rs:34,58` vs `onboarding.rs:14,24`).
- **API-client secrets in plaintext** — env/OAuth2 creds plaintext in SQLite; **history snapshots include substituted secrets**; global `OnceLock` cookie jar shared across users (`routes/api_client.rs:52-60`, `migrations/0014_api_client.sql`).
- **No single-instance guard / no port-conflict handling** (`main.rs:509`); fire-and-forget `tokio::spawn` with no panic supervision.
- **Migration safety** — forward-only, no down-migrations, no pre-migration backup; `0013_*` does a drop/rename rebuild without explicit `BEGIN/COMMIT`; some multi-step repo writes aren't transactional.
- **CORS `permissive()`** on all routes (`lib.rs:79`).
- **Workflow prompt injection** — upstream node output concatenated raw into agent prompts (`workflow_engine.rs:318`); HTTP nodes treat any status as success (`:349`).
- **`/meta` info disclosure (public)** — reveals `needs_onboarding`, provider list, tool versions (`routes/meta.rs:27-62`).
- **`providers` setting injection** — root can push arbitrary hot-reloaded provider config (`routes/settings.rs:36`) — potential RCE/cred-injection surface.
- **API surface inconsistency** — mixed workspace-scoped vs bare-id paths; `api_version` reported but no negotiation/deprecation.

### Recommendations
#### Must-have
- Add SSRF defense to all API-client outbound calls (resolve DNS, reject loopback/private/metadata, restrict redirects, gate TLS-skip per-request).
- Sandbox `fs.rs` to permitted roots; reject traversal/sensitive dotfiles; actually use `CurrentUser`/role.
- Require TLS (rustls) on the `0.0.0.0` listener; stop putting the bearer token in the WS query (use `Sec-WebSocket-Protocol` or a short-lived ticket).
- Add a startup reaper marking orphaned `running` workflow runs as `error`, plus a global per-run timeout.
- Scope notifications per-user (or per-workspace); stop broadcasting every `Notification` to all clients.
- Revoke all `auth_sessions` on password change/disable; add login rate-limiting/lockout.

#### Should-have
- Implement WS resync on reconnect (refetch authoritative state on `onopen`) + a monotonic event sequence and bounded replay buffer.
- Encrypt API-client env/auth secrets at rest (Keychain-derived key); redact secrets from history snapshots; partition the cookie jar per workspace/user.
- Enforce a password policy in `users::create/update`, not just onboarding.
- Validate the `providers` setting before hot-reload (close the config-injection/RCE surface).
- Wrap the destructive `0013` rebuild (and future data migrations) in explicit transactions; make `workspaces::create` transactional; snapshot the DB before migrating.
- Add single-instance enforcement (PID/lock file) + graceful port-conflict messaging.
- Tighten CORS from `permissive()` to an allowlist, especially with `network_listener` on.
- Bound concurrent workflow runs per workspace; JSON-escape upstream output before prompt injection; let HTTP nodes optionally fail on non-2xx.

#### Nice-to-have
- "Revoke all my sessions" endpoint + surface active sessions; add request timeouts/retries to the UI HTTP client.
- Reduce `/meta` disclosure for unauthenticated callers.
- Supervise long-lived background tasks (restart-on-panic) + a panic hook.
- Add an API versioning/deprecation policy; normalize route shapes to consistently carry workspace scope.
- Implement the stored-but-unexecuted pre/post request scripts in a sandbox, or remove the UI affordance.
- Periodic prune of expired `auth_sessions` + API-client history; scheduled `PRAGMA optimize`/`VACUUM`.

---

## 3. Agent Swarm

### Current state
Crate `otto-swarm` (persistence façade + pure logic): `service.rs` (`SwarmService` CRUD + `detail`/`graph`), `http.rs` (generic `router<S: SwarmCtx>`), `recruiter.rs`, `presets.rs` (5 embedded YAML org charts), `types.rs`. State: `otto-state/src/swarm.rs` `SwarmRepo` over 6 tables (`migrations/0029_agent_swarm.sql`); `ready_tasks` (Rust-side DAG filter), `children_complete`, `active_run_count`, `agent_has_active_run`, `stop_active_runs`, `board_for_agent`, `list_scheduled_agents`. Runtime in otto-server: `swarm_runtime.rs` (`coordinator_loop→tick→run_turn→route_result`; lifecycle; `CoordinatorRegistry`), `swarm_run.rs` (`run_turn`, resumed sessions, `parse_turn_result`, `CancelRegistry`), `swarm_workspace.rs` (`ensure_cwd` worktree/scratch/repo, `provision_agent`, `otto-post`), `swarm_scheduler.rs` (`is_due`, 60s tick), `routes/swarm_ingest.rs` (session-token gated). Reuses `agent_run`, `SessionManager`, `otto_context::materialize::provision`, `otto-git`. UI (`ui/src/modules/swarm/`): `SwarmPage`, `OrgTree`, `RunGraph`, `KanbanBoard`, `RunsList`, `BoardFeed`, `RecruiterWizard`, `NewSwarm`, `AgentEditor`; store handles 4 `swarm_*` events.

### Strengths
Clean reuse of existing agent machinery (no reinvented runtime); **genuine token efficiency** (resumed per-agent sessions, results read from out-file/transcript = 0 model tokens, distilled JSON payloads, headless `run_agent` only for recruit/plan); budget-bounded coordinator (`budget = cap - active_run_count`) + one-turn-per-agent + `todo→in_progress` claim; reasonably complete lifecycle (pause/abort/resume, coordinators restored on restart); DAG basics (`ready_tasks` deps-gating, recursive roll-up, cycle-guarded graph); recruiter validates skills/providers against the installed set; tolerant `extract_json` with tests.

### Gaps / Issues found
**Real bugs / correctness**
- **Backlog tasks never run** — see D2. `create_task` → `"backlog"`, `ready_tasks` selects `"todo"` only, no transition.
- **Worktree branch reset every turn** — see D1. `ensure_cwd` calls `worktree_add -B <branch> <base>` unconditionally → discards prior uncommitted work; no "already exists" guard.
- **Worktrees leak** — `worktree_remove` is never called; `<data_dir>/swarm/.../wt` and registry entries accumulate.
- **`delete_swarm` orphans the coordinator** — `http.rs::delete_swarm` deletes rows but never calls `stop_coordinator`; the loop ticks + warns forever.
- **No stale-run reconciliation on restart** — see D8. Unlike reviews/skill-evals, swarm runs left `running/waiting/queued` stay non-terminal, permanently consuming the cap and blocking the agent.
- **Tokens/cost never recorded** — `swarm_runs` has the columns and the plan says "pull from otto-usage," but nothing populates them; `RunsList` "Tokens" column is always `—`.
- **Scheduler↔coordinator↔manual TOCTOU** — `agent_has_active_run`/`active_run_count` are check-then-act with no lock/unique constraint; the 5s tick, 60s scheduler, and manual `run_task` can each create a duplicate run or exceed `cap`.

**Failure handling / guardrails**
- **No total-spend / run-count / wall-clock budget** — see D3. Only `max_parallel_sessions`. `route_result`'s `_` arm resets unknown/`in_progress` to `todo` → re-runs forever.
- **Infinite re-run on ambiguous results** — no per-task attempt counter.
- **Failed turn → `blocked` with no reason/notify/retry** path distinct from "stuck forever."
- **Handoff/review tasks have no dependency on their source** (`depends_on:[]`) → can be scheduled before the work completes.
- **`pick_agent` fallback assigns to "any active agent"** when keyword overlap is zero → wrong-role assignment.
- **`otto-post` requires `python3`** for JSON encoding; on a python-less machine board posts silently vanish (`>/dev/null 2>&1`).

**Observability / UX**
- **No live updates for org tree / projects / agents / new swarms** — only task/run/message/status events; other clients see stale data.
- **Graph not actually live** — `RunGraph` re-derives only on open/manual-refresh; shows tasks only (no run nodes); `parent_task_id` edges mislabeled `kind:"handoff"`.
- **No reconnect reconciliation; events for non-open swarms dropped** — `applyEvent` early-returns without caching; no WS-reconnect re-sync.
- **No blocked-reason visibility, no per-agent token totals, no cost panel, no dependency editor** (deps only set by the planner).
- **Pause/abort/resume are whole-swarm only** — no per-agent pause / per-task cancel.
- **Inconsistent destructive confirms** — Kanban delete confirms; OrgTree agent delete, "Abort all", "Plan from goal" don't; `swarm.loading` never rendered.
- **RunsList filters client-side** over a 500-row cap (backend filter params unused); **BoardFeed kind filter** omits several kinds.

**Testing**
- **Near-zero coverage** — only `recruiter.rs` (3) and `swarm_scheduler.rs` (2). Zero for `SwarmRepo`/`ready_tasks`/`children_complete`/`tick`/`route_result`/`run_turn`/`ensure_cwd` — all the bug-prone DAG/coordinator logic.

**Security**
- Board ingest is correctly session-token-gated. But `cwd_mode:repo` runs **directly in the project repo (no isolation)**; combined with no spend cap, an off-the-rails agent can edit the real repo with no guardrail.

### Recommendations
#### Must-have
- Add a `backlog→todo` path (or default created tasks to `todo`, or treat `backlog` as ready) — hand-added tasks currently never run.
- Add a swarm budget guardrail: `max_total_runs` / `max_cost_usd` / `max_runtime` enforced in `tick`, plus a per-task `max_attempts` — the autonomous, token-spending swarm has no off-switch but concurrency.
- Reconcile stale swarm runs on startup (mirror review/skill-eval recovery) before restarting coordinators.
- Fix worktree reuse: skip/`worktree add` without `-B` when the path exists — every turn currently wipes in-progress code.
- Stop the coordinator on `delete_swarm` (`stop_coordinator` + cancel runs + prune worktrees).
- Populate `tokens_input/output/cost_usd` from `otto-usage` in `run_turn`'s terminal persist.

#### Should-have
- Add a per-task attempt ceiling; stop re-queuing `in_progress`/unknown indefinitely (block + notify after N).
- Make handoff/review tasks depend on their source (`depends_on:[source_id]`).
- Add `swarm_agent_updated`/`swarm_project_updated`/`swarm_created` events + store routing; refresh the graph on task/run events; re-sync on WS reconnect; stop dropping events for non-open swarms.
- Prune worktrees on task done/abort via `worktree_remove`.
- Confirm on agent delete / abort / plan-from-goal; render `swarm.loading`; surface a blocked-reason + "re-run/unblock" action.
- Replace `python3` in `otto-post` with pure `sh`/`printf` JSON (or fail loudly).
- Guard `cwd_mode:repo` (warn/confirm, or default code roles to worktree).

#### Nice-to-have
- Per-agent pause / per-task cancel in addition to whole-swarm lifecycle.
- DAG/dependency editor in Kanban/graph; drag-drop between columns (currently menu-only).
- Cost/budget panel per swarm + per-agent token rollups; capacity-utilization indicator next to the cap.
- Stronger `pick_agent` (skill-match scoring; refuse unrelated auto-assign → leave unassigned + "needs assignee").
- Atomic claim via conditional `UPDATE ... WHERE status='todo'` (rows-affected check) to harden the TOCTOU.
- Server-side run filtering; inline run-error details; full board kind filter; graph collision-avoidance layout.
- Backfill tests for `ready_tasks`/`children_complete`/`route_result` delegation/roll-up/handoff routing.

---

## 4. Agent Sessions & Runtime (Sessions / Orchestrator / PTY / Connections)

### Current state
`otto-pty`: `portable-pty` wrapper; 80×24 PTY, blocking reader feeds a `vt100::Parser` (1000-line scrollback), a `RingBuffer` (10k lines / 2 MiB), and a `broadcast(1024)`; waiter thread reaps the child; `screen_snapshot()` emits a tmux-style coherent reattach. `otto-sessions`: `SessionManager` (`DashMap` of handles, viewer-count, suspend race-flag); 2s status task (Working/Idle from `last_output_at`, dead-handle eviction); idle-suspend sweep (`SUSPEND_GRACE=5min`, swept 60s) → `Reconnectable`, lazily resumed via `--resume` on attach; `trust.rs` pre-writes CLI trust; `prompt_guard.rs` auto-accepts known prompts; `ws.rs` terminal WS. `otto-orchestrator`: ⌘K planner driving a real claude PTY, scraping the JSONL transcript. `otto-connections`: per-kind command builders (secrets via env, except clickhouse argv), CRUD + Keychain + headless `test`. `agent_run.rs`: shared watch/retry loop. UI: `Terminal.svelte` (xterm + Fit/Search/WebGL, base64 I/O, capped-backoff reconnect, CSS-bidi RTL), `SessionView`, `Splits` (≤4), `TiledView` (all active).

### Strengths
Memory-efficient resume model (boot marks restorable → 0 RAM; idle-suspend frees ~200 MB/agent; deterministic kill↔exit race handling); snapshot-on-reattach avoids replay flicker; dead-handle eviction; conservative prune (only deletes positively-`Gone` transcripts); secret hygiene (passwords via env, Keychain); `AttachGuard` RAII; narrow `PromptGuard`; solid UI reconnect (terminal 500ms→5s, events 1s→30s); correctness-safe RTL (CSS bidi + font fallback, no stream rewriting, WebGL off in RTL).

### Gaps / Issues found
1. **Scrollback lost on reattach** — see D4. `screen_snapshot()` sends only the visible screen; client `lines` request ignored (`ws.rs:54-59,255-272`).
2. **TiledView wakes every session** — see D5. Resurrects all suspended sessions, no cap/visibility gating.
3. **Dropped SSH/DB sessions can't reconnect in place** — `ensure_live` only resumes agent sessions (`manager.rs:581`); `restart()` errors for connection sessions without `spec_override` (`:961-965`); UI opens a fresh tab, losing pane/DB-tab state.
4. **Unbounded auto-trust + skip-permissions** — every agent spawns with `--dangerously-skip-permissions`/`--dangerously-bypass-approvals-and-sandbox` (`providers.rs:71-111`); `trust.rs` writes `hasTrustDialogAccepted=true` into global `~/.claude.json`; `PromptGuard` auto-accepts residual prompts → fully unsandboxed, no per-workspace opt-out, no audit.
5. **No signal forwarding** — only write/resize/kill (SIGKILL); no graceful SIGTERM before kill on shutdown/suspend/restart; no programmatic interrupt (`pty/lib.rs:193-197`).
6. **Fixed 80×24 until first client resize** — headless runs (agent_run/orchestrator/review) run the whole turn at 80×24 → TUIs wrap/clip (`pty/lib.rs:20-21,62-67`).
7. **`last_output_at` idle detection is defeatable** — spinner TUIs never idle-suspend; quiet-but-busy agents flagged Idle at 5s (`manager.rs:1061-1065`).
8. **Broadcast lag drops output silently** — `Lagged` swallowed; the credential/prompt scanner can **miss an auth/trust prompt**, wedging an unattended session (`manager.rs:1042`, `ws.rs:203`).
9. **`screen_size` never pushed + no proactive snapshot on attach** — transient size mismatch + extra round-trip/blank (`ws.rs:172-272`).
10. **cwd→project-dir encoding mismatch** — `claude_pty::project_dir` (all non-alnum→`-`) vs `lifecycle::claude_project_dir_name` (only `/._`→`-`) can desync transcript reads vs resumability checks.
11. **Detached reader/waiter threads, no join** — a child ignoring SIGKILL leaks both threads for the process lifetime.
12. **Connection `first_command` = blind 1500 ms sleep** (`modules.rs:211`) — types into a not-ready shell if slow; needless latency if fast.
13. **`restart` doesn't set the `suspending` guard** → brief `Exited`→`Running` flicker (harmless).
14. **No server-side scrollback search, no recording/export, no split>4, no copy-on-select/linkifier.**

### Recommendations
#### Must-have
- Include scrollback in the reattach snapshot (prepend ring/parser history honoring `lines`) — history above the viewport currently vanishes on every reconnect.
- Cap/lazy-load TiledView terminals (IntersectionObserver or a live-tile cap) — currently defeats the idle-suspend memory design.
- Make the output scanner lag-proof for prompt detection (larger/unbounded queue, or re-scan screen text on `Lagged`) — a missed prompt stalls an unattended session.

#### Should-have
- Reconnect dropped connection sessions in place (rebuild the spec via `ConnectionsService::build_command`) so a dropped SSH/DB tunnel reopens in the same pane/tab.
- Graceful SIGTERM before SIGKILL on suspend/restart/shutdown; expose a signal/interrupt API.
- Spawn headless PTYs at a realistic size (e.g. 120×40) and/or send an initial resize.
- Push `screen_size` + an initial snapshot proactively on WS attach.
- Unify the cwd→project-dir encoding (one shared fn) so transcript reads and resumability always agree.
- Add a per-workspace trust/skip-permissions toggle + audit log for the `~/.claude.json` mutation.

#### Nice-to-have
- Smarter idle classification (diff the parser screen, not just `last_output_at`) so heartbeat TUIs can idle-suspend and quiet-busy agents aren't mislabeled.
- Readiness-based `first_command` instead of the fixed 1500 ms sleep.
- Terminal session recording/export (asciinema) + URL/file linkifier.
- Join/track the detached reader/waiter threads (or async PTY read) to avoid thread leaks under churn.
- Set a `suspending`-style guard during `restart` to remove the status flicker.
- Make xterm scrollback (10k) and PTY ring caps configurable; add server-side ring search.

---

## 5. Database Explorer

### Current state
TablePlus/Navicat-class browser for MySQL/Redis/MongoDB/ClickHouse over plaintext/TLS/SSH. Backend (`otto-dbviewer`, ~10K LOC): engine-agnostic `Driver` trait + stateless `Registry`; `DbViewerService` resolves a stored connection + keychain secret, opens/reuses SSH tunnels (`-L` for SQL/Redis, `-D` SOCKS5 for Mongo/Atlas), dispatches, records history; REST router gated by `Viewer`/`Editor`; auto-LIMIT in `types::inject_row_limit`. Frontend (`ui/src/modules/database`, ~7.9K LOC): virtualized `ResultsGrid` (windowed rows, client filter/sort, CSV/TSV/JSON export, review-gated inline edit/delete/duplicate), visual JOIN builder, Superset-style ClickHouse dashboards/widgets, saved queries + history, "send result to agent."

### Strengths
Clean engine-agnostic contract; stateless drivers with per-`cache_key` connection cache + idle eviction (600s); conservative, well-tested `inject_row_limit` (only `SELECT`/`WITH`/`(SELECT`, skips multi-statement/existing-LIMIT/UNION/FORMAT/etc., leaves SHOW/DESCRIBE/EXPLAIN alone); review-gated inline edits (UI builds the exact statement shown in an editable textarea = source of truth; refuses no-PK tables / PK-not-in-SELECT; composite-key support); real grid virtualization; JOIN builder quotes identifiers + excludes unreachable tables; keychain-backed secrets never serialized; SSH `BatchMode=yes` + `ExitOnForwardFailure=yes` + `kill_on_drop` + Drop reaping.

### Gaps / Issues found
- **Cancellation is client-only** — `abortQuery()` only aborts the fetch; no cancel/KILL endpoint or driver cancel; a heavy query keeps running and holds the cached connection (`database.svelte.ts:1028-1038`).
- **`run_widget` privilege escalation** — see S7. Viewer can run arbitrary SQL (incl. writes/DDL) via a stored widget statement (`http.rs:469-479`).
- **No backend read-vs-write/destructive gating** — `run_query` runs `DROP`/`TRUNCATE`/unqualified `DELETE`/`deleteMany({})` at `Editor` with no confirmation; approval is purely a UI convention (`http.rs:249-258`).
- **Client-built SQL escapes only single quotes, not backslashes** — `sqlLiteral`/`valueLiteral`/`whereByPk` (`ResultsGrid.svelte:641-653`) and filter compilation (`database.svelte.ts:85-109`); MySQL default mode treats `\` as escape → malformed/injectable literals on inline edit / quick-filter. ClickHouse server-side `esc()` has the same backslash gap (Low there).
- **Full result buffered before truncation (MySQL)** — `run_read` does `fetch_all` then truncates (`mysql.rs:677-690`); with the "All" sentinel (`ROW_LIMIT_ALL=1_000_000`) or any LIMIT-skipped statement the whole set materializes. ClickHouse streams (good); Mongo `find` caps but `aggregate` does not.
- **Mongo aggregate bypasses the server-cost cap** — `find()` injects `.limit`, but `aggregate()` runs the raw pipeline and only caps returned docs client-side (`mongodb.rs:403-406`).
- **Export in-memory only; no streaming, no import** — covers already-fetched LIMIT-capped rows; no server-streamed full-table export, no import path.
- **TLS verification can be disabled on every engine** (`verify=false` → `danger_accept_invalid_certs`/`allow_invalid_certificates`/`insecure`); `Preferred` allows silent downgrade; no UI warning.
- **Client-cert/CA private keys written world-readable to `/tmp`, never cleaned** — see S8. `tls.rs:16-26`.
- **Local-port TOCTOU in tunnels** (low) — `free_local_port` binds `:0`, drops, then spawns `ssh -L` (`tunnel.rs:138-148`).
- **DB Explorer endpoints undocumented** — `api.md` contains none of the `/connections/{id}/db/*`, saved-query, dashboard, or widget routes.
- **Minor:** best-effort/swallowed history writes; `inject_row_limit` misses backtick-quoted `` `limit` `` and `/* LIMIT */` cases (→ full scan); **Postgres entirely absent**.

### Recommendations
#### Must-have
- Add a server-side cancel endpoint issuing engine-native cancellation (MySQL `KILL QUERY`, ClickHouse `KILL QUERY WHERE query_id=`, Mongo `killOp`/cursor drop) and wire `abortQuery` to it.
- Gate `run_widget` to require `Editor` (or reject non-read statements on the Viewer path).
- Backslash-escape + quote client-built SQL literals/identifiers (`ResultsGrid.svelte:641-653`, `database.svelte.ts:85-109`).
- Chmod TLS temp PEMs to `0600` and clean them up on teardown (`tls.rs:16-26`).

#### Should-have
- Add an explicit backend destructive-op confirmation flag (or classify statement kind) so `DROP`/`TRUNCATE`/unqualified `DELETE`/`deleteMany({})` can't run un-reviewed outside the UI modal.
- Inject a leading `$limit`/wrap aggregate pipelines and bound the "All" sentinel so Mongo aggregate and the 1M-row option don't do unbounded server work.
- Stream large MySQL reads (`fetch` not `fetch_all`) + a server-streamed full-table export.
- Surface a UI warning when `tls.verify=false`/`Preferred`; pin `StrictHostKeyChecking=accept-new` + explicit `UserKnownHostsFile` (`tunnel.rs:154-173`).
- Update `api.md` with the full DB Explorer route surface.

#### Nice-to-have
- Add CSV/JSON import into a table; add a Postgres driver (the types are already engine-agnostic).
- Harden `inject_row_limit` for backtick-quoted `` `limit` `` and `/* … */`-hidden LIMIT cases.
- Add RTL/logical-property CSS to the grid; de-dup widget auto-refresh vs manual refresh.
- Make Redis keyspace truncation hints actionable; implement native-transport active-DB scoping for ClickHouse.

---

## 6. Git, Pull Requests & AI Code Review

### Current state
Shells out to system `git` (`otto-git/src/local.rs`, `GIT_TERMINAL_PROMPT=0`), parses plumbing with pure functions (`parse.rs`: porcelain-v2, diff3 conflict segments), ~40 REST routes (`http.rs`): status/branches/refs/log/diff/stage/unstage/discard/commit/push/pull/checkout/stash, local merge + conflict resolution, hosted-PR ops. Three providers (`github.rs`/`bitbucket.rs`/`gitlab.rs`) over a common `GitProvider` trait + retrying client; remote-URL→provider detection (https/ssh/scp, nested GitLab groups). Creds via a temp `GIT_ASKPASS` script. PR-draft (`modules.rs::draft_pr`) and AI review (`run_review_core` + `review_session.rs`) build prompts from a diff, fan out N reviewers (lens × provider) as openable sessions writing findings JSON, then a claude summarizer dedupes/ranks. UI (`ui/src/modules/git/`): repo browse, collapse-aware `DiffViewer`, whole-file `ChangesView`, `CreatePr` "Draft with AI", history/graph, per-hunk `ConflictResolver`, `ReviewPanel`/`LocalReviewPanel` (2s polling, per-agent retry).

### Strengths
Correct-by-construction merge (never `-X ours/theirs`, `diff3` style, refuses dirty start, returns conflicts as a result not an error); conflict segments round-trip deterministically (tested); per-repo async locks for merge/conflict; per-index review persistence (concurrent agents don't revert each other); resilient review agents (3-attempt recovery, stuck/timeout detection, strong anti-prompt-injection system prompt); push auto-sets upstream; diff parsing handles renames/binaries/new-deleted/`\ No newline`/unborn HEAD; `DiffViewer` auto-collapses >400-line files + lazy-loads highlight.js; confirmed discard.

### Gaps / Issues found
- **Worktree/submodule merge break** — see D9. `merge_source` hardcodes `repo_path/.git/MERGE_MSG` (`local.rs:669-670`); a linked worktree/submodule has `.git` as a *file* → falls back to a raw SHA. Reachable via the swarm.
- **Untracked files invisible to review & PR-draft** — see D10. `diff_text_against` = `git diff <base>` (`local.rs:335-337`) excludes untracked/new files; the richer `DiffTarget::Working` isn't used.
- **Symlink escape in conflict resolve** — see S11. `safe_join` doesn't resolve symlinks (`local.rs:756-773`).
- **`HistoryView.loadMore()` swallows errors** — no try/catch → silent dead "Load more" on any transient failure (`HistoryView.svelte:40-46`).
- **Summarizer is the only dedup and fails open** — one claude call, fixed 120s; on failure it naively concats batches (`agent_outputs.join(",")`) with no dedup (`modules.rs:1298-1310`); 120s tight for large batches; no frontend fallback.
- **Fork / multi-remote / cross-repo PRs unsupported** — `create_pr` always same-repo `head: source_branch` (no `owner:branch`); UI assumes `origin`, no fork/upstream picker (`CreatePr.svelte`, `github.rs:250-264`). Relevant to the Bitbucket fork workflow.
- **No hunk/line-level staging** — backend `stage` + `ChangesView` are whole-file only.
- **Clone is always full** — no `--depth`/`--filter`/`--single-branch`, no `--recurse-submodules`, no LFS; progress at debug only, no percentage (`local.rs:849-926`).
- **`Working` diff spawns one `git diff --no-index` per untracked file serially** — O(n) subprocesses, no size guard (`local.rs:305-313`).
- **No diff virtualization** — expanded files mount all rows; uncapped whole-PR diffs jank low-end/mobile (`DiffViewer.svelte`; PR diff fetched uncapped `modules.rs:1389`).
- **No concurrency guard on starting reviews** — repeated `start_review`/`start_local_review` pile up agent CLIs (`modules.rs:1653,1843`).
- **Detached HEAD unhandled in UI** — no badge; `current_branch` returns `"HEAD"` while `parse_status` yields `"(detached)"`.
- **Bitbucket `request_changes` ignores the body** (`bitbucket.rs:376-381`); approve/decline lack precondition checks; GitLab semantics surfaced identically to GitHub.
- **Secret leakage** — findings/diff snippets stored/shown verbatim with no redaction guidance; handoff re-sends diff context.
- **Doc drift** — `api.md` documents git/PR rows only through #56; merge/conflict/refs/fetch/discard and the entire review/draft/handoff surface are undocumented.

### Recommendations
#### Must-have
- Fix `merge_source` to use `git rev-parse --git-path MERGE_MSG/MERGE_HEAD` (worktree/submodule-safe) — `local.rs:669-670`.
- Include untracked + full staged-new files in local-review and PR-draft (reuse `DiffTarget::Working`) — `local.rs:335-337`, `modules.rs:1494,1857`.
- Resolve/reject symlinks in `safe_join` (canonicalize, confirm under repo root) before `write_resolution` — `local.rs:756-773`.
- Wrap `HistoryView.loadMore()` in try/catch; surface the error and stop the spinner — `HistoryView.svelte:40-46`.

#### Should-have
- Robust summarizer dedup: scale timeout with batch size; on failure do a deterministic key-based merge `(path,line,normalized-body)` — `modules.rs:1298-1310`.
- Support fork/cross-repo PRs: accept `head: owner:branch` + target namespace/remote (`CreatePr.svelte`, `github.rs:250`, `bitbucket.rs:277`).
- Guard against duplicate concurrent reviews per PR/repo (`modules.rs:1653,1843`).
- Cap/virtualize very large diffs in `DiffViewer` + cap PR-diff fetch (`DiffViewer.svelte`, `modules.rs:1389`).
- Add redaction guidance to reviewer + summarizer prompts; mask obvious secrets in stored findings/diff snippets.
- Surface a detached-HEAD badge; normalize `current_branch` "HEAD" → "(detached)"/short-SHA.
- Batch the untracked-file diff (single render or skip over a size threshold) — `local.rs:302-313`.

#### Nice-to-have
- Add hunk/line-level staging (`git apply --cached --recount` + UI hunk checkboxes) — `local.rs:370-378`.
- Offer shallow/partial clone + submodule/LFS handling + a real clone progress % (`local.rs:849-926`).
- Add a finding-triage layer (dismiss/severity filter, mark false-positive feeding back into config) + a frontend dedup backstop.
- "AI resolve this conflict" action wiring the conflict file to an agent.
- AbortControllers/request-sequencing on git status/diff/conflict polls.
- Update `api.md` to cover merge/conflict/review/draft + remaining git routes.
- Make the review grace period + reviewer/summarizer timeouts config-driven (`modules.rs:1029-1038,1304`).

---

## 7. Product (Jira/Confluence), Issues & Channels (Slack/Telegram)

### Current state
`otto-issues`: thin per-account REST clients — `JiraClient` (v3) + `ConfluenceClient` (v1) from `base_url`+`email`+`token` (Basic); projects/JQL/full-issue/comments/transitions/assign/attachment-proxy/create/update, page get/create/update + storage-XHTML↔Markdown converters; `http.rs` axum router (token in `SecretStore` `issueacct-<id>`). `otto-product`: `ProductService` wires import→version→questions→notes→testcases→learnings→inject→discovery (publish RFC/Jira story); `skills.rs` seeds 7 skills; `http.rs` workspace-role-checked. `otto-server`: `product_run.rs` multi-provider analysis fan-out (each lens×provider a real session, bracketed-paste with verify-and-repaste, JSON out-file, summarizer consolidates); `product_watcher.rs` polls watched stories 60s (floored 5min), records new comments, advances `watch_cursor`, runs a retrying reconcile agent. `otto-channels`: `ChannelManager` supervises Slack Socket-Mode + Telegram long-poll (re-scan 15s); `Bridge` maps `(workspace,chat,thread)`→one session; `Mirror` tails the transcript to a throttled feed + posts the final reply; tokens `chan-bot-*`/`chan-app-*`.

### Strengths
Provider-honoring fan-out (real openable sessions, per-agent retry/stuck-detection/cancel, dropped-paste repaste loop); good auth hygiene on **product** routes (every handler role-checks the workspace; attachment proxy keeps the token server-side); watcher safety (5-min floor, cursor advanced before reconcile, 3× retry, story-scoped question updates); channel resilience (Slack acks immediately, dedup by `channel:ts`, ping/close/reconnect; Mirror throttles edits 2.5s, trims to char budgets; bot-self-event loop prevention); Confluence update is optimistic-concurrency aware; test-case republish reuses the page id; suggested learnings created inactive; test cases require explicit PO approval before publish.

### Gaps / Issues found
**Security / authorization**
- **(Critical) Issue accounts not ownership-checked on read/use** — see S4. Only `create/update/delete` verify `account.user_id`; all reads/uses take any `account_id` (`http.rs:205-499`).
- **Product service uses `story.account_id` with no owner/workspace tie** — the issue account is effectively a global credential.
- **`token_expires_at` stored but never enforced** — expired tokens surface as opaque 401s.

**Jira/Confluence API robustness**
- **No timeouts, retries, or 429/`Retry-After` handling anywhere** — bare `reqwest::Client::new()` (`jira.rs:156`, `confluence.rs:80`, `slack.rs:53`, `telegram.rs:72`); a hung endpoint hangs the request and stacks in the watcher.
- **No pagination** — `list_projects` caps 100, `list_spaces` 200, search 25, `list_comments` reads only page 1 (`jira.rs:467`, `confluence.rs:441`) → watcher silently misses comments past page 1 then advances the cursor past them.
- **Watcher refetches ALL comments every poll then filters client-side** (`service.rs:331-375`); lexicographic RFC3339 cursor breaks across TZ offsets.
- **JQL/CQL injection surface** — `escape_jql` only escapes `"` (`jira.rs:1492`); `space_key` interpolated into CQL unvalidated (`confluence.rs:527`).
- **Hand-rolled XHTML↔Markdown is lossy/fragile** (`confluence.rs:614-1141`) — tables/panels/`ac:*` macros/images dropped; round-tripping a real page can silently destroy content.
- **`add_comment` uses the deprecated `type:comment` Confluence endpoint** (`confluence.rs:225`); Jira `text_to_adf` drops markdown structure.

**Publishing safety / idempotency**
- **No idempotency on publish** — `publish_as_rfc` always `create_page`, `publish_as_story` always `create_issue` (`service.rs:1344,1434`) → double-click/retry creates duplicates.
- **Confluence lost-update window** — get-then-update with no 409 retry (`service.rs:529`, `confluence.rs:177`).
- **Publish records the "published" event even if the best-effort cross-comment failed** (`service.rs:766,1540`).
- **UI lacks confirmation gates + double-submit guards** for external publishes (`PublishDialog.svelte`, `TestCasesTab.svelte`, `RewriteTab.svelte`).

**Watcher polling vs webhooks**
- Polling-only (no webhook option) → ~5-6 min latency + constant full refetch; `last_poll` is in-memory → restart re-polls every story; reconcile + self-improvement run on every comment batch unconditionally (token-expensive).

**Channels reliability**
- **Slack uses the deprecated/sunset `files.upload`** (`slack.rs:137`); Telegram `sendDocument` uploads `from_utf8_lossy` → corrupts binaries (`mirror.rs:472`).
- **No attachment size limits** — whole files read into memory (`slack.rs:511`, `mirror.rs:470`).
- **No per-conversation serialization** — `Bridge::handle` spawns per message (`bridge.rs:480`); two quick messages race the same PTY.
- **Telegram resets `offset=0` on restart**, no `allowed_updates`, no dedup, drops every non-text update (photos/docs/captions) — Telegram inbound files entirely unsupported (`telegram.rs:218,262`).
- **No backoff/jitter on reconnect** (fixed 3-5s); Telegram ignores `429 retry_after`.
- Mirror final-reply dedup is content-based → drops a legitimately-repeated reply; Slack dedup `clear()` on overflow can re-admit a just-seen event.
- **`allowed_users` empty ⇒ ANY chat user drives an agent** with full tool access (`bridge.rs:266`).

**Multi-provider orchestration & correctness**
- Agent output is best-effort JSON scraping (`extract_json_block`, `product_run.rs:400`) — a prose/near-miss reply silently counts as "no findings"; no re-ask.
- Context file written to shared `std::env::temp_dir()` (`product_run.rs:716`), world-readable, with private Jira data, left on disk.
- Summarizer single-provider, no fallback.
- `is_issue_key` precedence bug accepts lowercase after the first char (`jira.rs:1477`); Epic-link hardcoded to `customfield_10014` (`jira.rs:1127`); `get_attachment` can 401 on the CDN redirect hop (reqwest strips `Authorization` cross-host); product UI renders external content via `{@html renderMarkdown(...)}` as the sole XSS defense.

### Recommendations
#### Must-have
- Add account-ownership (or workspace-scope) checks to every issue read/use handler (`otto-issues/src/http.rs`) — closes the cross-user credential leak (highest-risk).
- Set connect+request timeouts on every `reqwest::Client` (Jira/Confluence/Slack/Telegram).
- Add idempotency to `publish_as_rfc`/`publish_as_story` (track published page-id/issue-key or accept an idempotency key) — `service.rs:1344,1434`.
- Replace Slack `files.upload` with `getUploadURLExternal`+`completeUploadExternal`; upload attachments as raw bytes not `from_utf8_lossy` — `slack.rs:137`, `mirror.rs:472`.
- Handle 429/`Retry-After` with bounded backoff in all clients.

#### Should-have
- Paginate `list_comments`/`list_projects`/`list_spaces`/`search` — watcher silently misses comments past page 1.
- Switch the watcher to a server-side "comments since timestamp" query + persist `last_poll` across restarts.
- Serialize messages per `(workspace,chat,thread)` (queue/mutex) — `bridge.rs:480`.
- Validate lens output against schema + re-ask once on a parse miss; surface conflict notes — `product_run.rs:400,843`.
- Add 409-retry to Confluence `update_page` callers.
- Per-publish confirmation modal + hard double-submit guard in the UI.
- Enforce `token_expires_at` with a pre-flight "re-auth needed" error.
- Cap attachment sizes before reading into memory.
- Delete the temp context file after the fanout + restrictive perms (private Jira data).
- Require a non-empty `allowed_users` (or explicit "open" opt-in) before a channel can drive sessions.

#### Nice-to-have
- Offer Atlassian webhook ingestion (Jira `comment_created`/`issue_updated`, Confluence) instead of 60s polling.
- Replace the hand-rolled XHTML↔Markdown with a tested converter (or round-trip-guard tables/panels/macros).
- Reconnect backoff with jitter; honor Telegram `retry_after`; persist Telegram `offset`.
- Support Telegram inbound file attachments (`getFile`).
- Make the Epic-link field id + summarizer provider configurable; add a summarizer fallback.
- Fix `is_issue_key` precedence; add fuzz/property tests for JQL/CQL escaping.
- Add a secondary HTML-sanitization pass (DOMPurify/CSP) behind the `{@html}` sites.
- Virtualize large lists (changelog, questions, test cases, learnings).

---

## 8. Usage & Cost, Insights, Self-Improvement & Skills Library

### Current state
- **Usage** (`otto-usage`): embedded ClickHouse (`clickhouse local --path`, no server) tails Claude (`~/.claude/projects/*/*.jsonl`) + Codex (`~/.codex/sessions/.../rollout-*.jsonl`); daemon tailer (`ottod/src/usage_tailer.rs`) tracks per-file byte offsets (atomic tmp+rename), inserts into a `MergeTree` `usage_events` with `TTL event_date + N DAY`; rollups by provider/day/session; `/ingest/usage` path; root-gated `/usage/*` UI with input/output + cache read/write breakdown.
- **Insights** (`insights.rs`): opt-in per-cadence (daily/weekly/monthly, default off); hourly-gated supervisor spawns a headless agent running the `insights` skill, writing `report/summary/metrics` + `index.json`; idempotent by artifact presence; root-gated writes.
- **Self-improvement** (`otto-improve`): off-by-default, workspace-scoped; prompt from session digests + skills/memory → multi-provider producer → `ImprovementProposal` → per edit resolve guarded path, classify, apply (whole-file write) or queue; Tiered autonomy (allow-listed Memory/Skill + `Low` risk auto-applies); approve/reject/rollback with conflict checks + full audit trail; triggers: scheduler/live/product-narrative.
- **Skills** (`otto-skills`, `skill_eval.rs`): bundled via `include_dir!`, versioned by `version:` frontmatter; `install_state` NotInstalled/UpToDate/UpdateAvailable/Ahead; install/update copies the tree (root-only, backup-before-install); the Skills Evaluator runs a skill through a real impl agent in a worktree, fans out N validators×providers + an improver across iterations, can promote the best iteration.

### Strengths
Real per-turn usage from tailing (partial-line safety, truncation reset, EOF-seeding, atomic cursor; cache read/write surfaced); insights period math pure + unit-tested + path-confined; self-improvement has genuinely good safety scaffolding (strict `pathsafe.rs`, workspace allow-list, off-by-default, admin-only, full audit + rollback, dual single-run guards); skills immutable in-binary, root-only install with timestamped backup.

### Gaps / Issues found
**Usage & cost**
- **Cache tokens never priced** — `estimate_cost(model, input, output)` ignores cache read/write (`pricing.rs:8`); for agentic loops cache reads (~0.1× input) and writes (~1.25× input) dominate. Cost materially understated.
- **Pricing table stale & fragile** — substring match: Opus `(15,75)` vs current $5/$25 (~3× over-bill); Haiku `(0.80,4.0)` vs $1/$5; **Fable 5 / Mythos 5 match nothing → silently $0**; any unknown/new model → $0, no diagnostic; no date stamp, no fallback (`pricing.rs:11-23`).
- **No DB-level dedup + double-count windows** — plain `MergeTree`, no idempotency key; the cursor file is the sole guard → crash-after-insert-before-`save()` double-counts; no inode tracking.
- **Silent skips** — JSON parse failures `continue` with no log/metric; possible daemon-vs-ClickHouse TZ skew in `today()-N` math.
- `total_tokens` sums input+output+cache 1:1, inflating "volume" vs cost.

**Insights**
- **Cache invalidation/staleness** — `period_done` treats any artifact's presence as done → a partial run (metrics but no HTML) is "complete" forever.
- **Fire-and-forget runs** — the spawned session is never awaited/checked; the in-flight slot releases after a fixed sleep; a crashed/looping skill leaves no failure signal.
- Usage/insights/improve/skills routes undocumented despite "FROZEN" `api.md`.

**Self-improvement (highest risk)**
- **Prompt injection from session content** — see S6. `prompt.rs:64-74` interpolates raw `d.text`/`d.title` (untrusted transcript text) with no escaping; `run_for_narrative` feeds Jira/Confluence text the same way.
- **Risk tier is LLM self-reported and fully trusted** — disposition keys off `edit.risk`/`target_type` from the model's JSON; Memory edits bypass the allow-list, so Memory + self-labeled `Low` + default Tiered → auto-write (`classify.rs`, `engine.rs:356`); `patch.after` written as the **entire file** with no validation/size cap → persistent `MEMORY.md` poisoning.
- **`run_for_narrative` supplies its own allow-list** (`target_skills` as both candidate set and allow-list, `engine.rs:534,587`) → externally-triggered narrative runs auto-apply skill edits.
- **Auto-apply has no pre-flight conflict check and no atomic write/backup** (unlike approve/rollback) → clobbers concurrent edits, can corrupt files; no `canonicalize()`.

**Skills**
- **Update destructive, integrity = version number only** — `install_into` does remove-dir-then-copy (non-atomic; mid-copy failure orphans the skill) with no checksum; the `Ahead` (locally-modified) state is shown but **not enforced** → update overwrites; an improve-edited installed skill is later clobbered by a bundled "Update" (`lib.rs:161`).
- **Skill-eval resource leaks & unbounded cost** — cleanup runs only on cancel/delete, **not on normal completion** (`skill_eval.rs:874`) → successful runs leak live sessions + `/tmp/otto-skilleval/...` worktrees; no cost budget (impl 40m + N×providers×passes 15m + improver 10m × iterations); archive temp dirs not cleaned.

### Recommendations
#### Must-have
- Price cache tokens: change `estimate_cost` to take cache read/write + per-model cache rates (read ≈0.1×, write ≈1.25×); update both call sites + the tailer (`pricing.rs`, `routes/usage.rs:240,308`).
- Refresh + restructure the pricing table to current pricing (Opus $5/$25, Haiku $1/$5, add Fable 5/Mythos 5 $10/$50, Sonnet $3/$15) + a non-zero fallback for unknown models + a `priced_as_of` date (`pricing.rs`).
- Sanitize/escape session digest text in the improve prompt (`serde_json::to_string` or hard sentinels) + an explicit "session content is untrusted" guard (`prompt.rs`).
- Stop trusting LLM-self-reported risk for Memory auto-apply — gate behind a deterministic check (size cap + reject injection/role markers, or route through the same allow-list/approval as skills) (`classify.rs`, `engine.rs:356-373`).

#### Should-have
- Add a ClickHouse idempotency guard (`ReplacingMergeTree` on `(provider,session_id,ts,…)` or a content hash) + order cursor persistence with insert; track inode alongside path (`schema.rs`, `usage_tailer.rs`).
- Make improve auto-apply atomic + conflict-checked (compare to `before_content`, tmp+rename, keep a backup) — `engine.rs:358-373`.
- Run skill-eval cleanup on normal completion too (`archive_eval_sessions` + `remove_eval_worktrees`); clean archive temp dirs; add a concurrency/cost cap (`skill_eval.rs:874,1671-1710`).
- Enforce the skills `Ahead`/locally-modified state (block or confirm before overwrite) + a content hash; make `install_into` atomic (copy to temp then rename) — `lib.rs:161-192`.
- Track insights run success + re-run on partial artifacts (require the HTML / an explicit `done` marker); surface failures instead of fire-and-forget (`insights.rs:215-224,440-455`).

#### Nice-to-have
- Log/metric transcript parse failures + unknown-model `$0` pricing events.
- Document usage/insights/improve/skills/skill-eval routes in `api.md`.
- Add disk-growth/TTL-merge health to `/usage/status`.
- Make usage rollup timezone explicit (fixed TZ) to avoid date-boundary skew.
- `canonicalize()` improve target paths before write (symlink defense-in-depth).

---

## Appendix — Method & confidence notes

- Produced by 8 parallel read-only audits; each independently read the relevant backend + UI + contracts and (for several sections) cross-checked with secondary explorer passes. Where sub-passes disagreed, the more precise line-referenced finding was kept and over-stated claims were corrected (e.g. the ClickHouse `esc()` "critical injection" was downgraded to a Low backslash gap; the DiffViewer was confirmed to already auto-collapse large files).
- Every recommendation references concrete files/symbols so each can be turned directly into an issue/ticket. Line numbers reflect the working tree at audit time and may drift.
- This is an audit, not a verification run: findings were derived from code reading, not by executing exploits. Validate severity before acting on the security items — but the remote-exposure set (S1–S5, S9) should be treated as blocking for any non-loopback deployment.
