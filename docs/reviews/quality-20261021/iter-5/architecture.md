# Architecture review: quality loop 2026-10-21, iteration 5

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

The review was read-only, using `git log`, `git diff`, `git archive` (into a scratch directory, for
the iteration-4 import matrix), `rg`/`grep`, `sed`, `awk`, `wc` and local Python imports of
`scripts/loc-ratchet.py` (its own `production_lines`, `function_lengths` and `is_test_file`). Nothing
was built or tested. Every finding was confirmed against `fix/quality-iter5-20261022` at `8177c4d85`
(44 commits after the iteration-4 snapshot `6cff08270`, including the MCP profiles, Jira create
issue, room games and classroom merges).

Excluded: `target/`, `node_modules/`, `docs/reviews/`. Unlike iteration 4, the room games
(`ui/src/modules/rooms/games/**`, `otto-server/src/game_rooms/`) and the classroom
(`ui/src/modules/home/school/**`, `ClassroomsBox.svelte`) are in scope.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Earlier findings still open, fully or as a residual | 0 | 4 (1-04, 1-09, 1-10, 4-02) | 7 (1-01r, 1-05, 1-07r, 1-08r, 1-13, 2-06, 2-08) | 4 (2-01r, 3-03r, 4-01r, 4-04r) |
| New in iteration 5 | 0 | 0 | 4 (5-01, 5-02, 5-03, 5-04) | 1 (5-05) |
| **Total** | 0 | 4 | 11 | 5 |

**Score: 10.0 − 4 × 0.3 − 11 × 0.1 − 5 × 0.03 = 10.0 − 1.2 − 1.1 − 0.15 = 7.55 / 10**

The score went up from 7.31 to 7.55. Iteration 4 fully fixed 1-16, 3-02r, 4-03 and 4-05, fixed 3-03
in substance (a nit residual remains), and turned the 4-01 major and the 4-04 minor into nit
residuals. The new findings are smaller than last time: no new major.

To reach 9.8, the remaining work is, by weight:

- the four majors: the two UI god files (1-09 GraphView, 1-10 DatabaseStore), the shared fan-out
  runner (1-04) and the assist-engine merge (4-02 phase 2). 5-02 (move `agent_session` into
  `otto-agent-run`) is the enabling step for both 1-04 and 4-02;
- the room relay extraction, now unblocked by the games merge: 2-06, 2-08 and 5-01 are one move.

## Status of earlier findings

| ID | Sev (then → now) | Status | Evidence now |
|----|------------------|--------|--------------|
| 1-01 (residual) `run_workflow_with_budget` | minor | Open | `workflow_engine.rs:1850`, still 1,250 production lines, capped by `loc-baseline.json` `functions.run_workflow_with_budget: 1250`. FIXES P02 "not started". |
| 1-04 fan-out + summarizer per feature | major | Open (deferred in SCORES.md) | `skill_review.rs:400` (`agent.name == "summarizer"`), `vault_docs_agent.rs:356` (`SUM_RETRY_IDX = usize::MAX`), `modules/review/summarize.rs` (`summarize_and_persist` 478 lines, now ratcheted). `vault_docs_agent.rs:14` still says it "Mirrors the `product_run` fan-out + summarizer". `otto-agent-run/src` has no fan-out runner. |
| 1-05 two MCP surfaces | minor | Partly fixed, still open | **Fixed:** the native schemas are generated from the control-plane catalog (`ottod/src/mcp_tools/listing.rs:1-17`); `DEFERRED_SCHEMA_DRIFT` is empty (`catalog_tests.rs:13`). **Open:** the two executors still map the same tool to the same daemon route twice. Example: `otto.list_prs` → `exec.rs:626` builds `/api/v1/repos/{repo}/prs?…`; `otto_list_prs` → `mcp_tools.rs:2635` builds `/repos/{repo}/prs?…` with its own `opt_query`. `route_for` is 1,555 lines, `run_tool` 1,189, `read_route` 446 and `base_tool_catalog` 687, all ratcheted. MCP profiles added a sixth per-tool table (`otto-mcp/src/profiles/mod.rs:54` `GROUPS`), test-guarded. |
| 1-07 (residual) REST engine in a route file | minor | Open | `routes/api_client.rs` (3,266 lines) still defines `execute` (`:1808`), `execute_service` (`:1826`) and `resolve_environment` (`:2017`). Non-route callers: `workflow_api_request.rs:50`, `routes/api_automation_runs.rs:103`, and `modules/browser_proxy.rs:117,229` + `routes/api_oauth.rs:120,286` through the `routes::api_client::net_guard` re-export (`:86`). FIXES P03 "not done". |
| 1-08 (residual) `WorkflowsPage.svelte` | minor | Open | 3,710 lines (was 3,703). `refetchRun` `:285`, `waitRunTerminal` `:1003`, `approveRun` `:1445` still in the page. |
| 1-09 `GraphView.svelte` | major | Open | 4,887 lines (was 4,877), capped at 4,887 in `ui-guards-baseline.json:180`. 18 raw API calls. FIXES P04 "not started". |
| 1-10 `DatabaseStore` | major | Partly fixed, still open | 4,423 → 3,661 lines (`model.ts`, `statement.ts`, `dashboards.svelte.ts` extracted). The class (`database.svelte.ts:125`) still holds about 115 methods across assist (`:402-562`), access changes (`:591`), tabs (`:711-952`), connection warm/keep-alive (`:1555-1862`), SSH tabs (`:1889-1910`), schema tree (`:2089-2240`), object ops (`:2286-3215`), query runs and plans (`:2790-3006`), filters (`:3242-3318`), saved queries (`:3402-3524`) and history (`:3542-3582`). |
| 1-13 `lib/` imports `modules/` | minor | Partly fixed, still open | 33 → 21. `database.svelte.ts` has none now. Left: `uiCommands/k8s.ts` (3), `uiCommands/home.ts` (2), `stores/swarm.svelte.ts` (2), `stores/canvas.svelte.ts` (2), `router.svelte.ts` (2), and 10 single imports. |
| 1-16 usage tailer in the binary | minor | **Fixed** | `ottod/src/usage_tailer.rs` is gone; the driver is `otto-usage/src/tailer/driver*`. One Codex model reader (`otto-transcript/src/codex.rs:1083`). |
| 2-01 (residual) terminal grid negotiation | nit | Open | `deferLocalResize` (`Terminal.svelte:1108`) and `sendResize` (`:1324`) still beside `TermGridSync` (`:1309`). |
| 2-06 rooms inside otto-server | minor | Open | `otto-server/src/rooms/` + `game_rooms/` = 8,210 lines, 12 commits in 3 weeks. The deferral reason ("blocked on the games team's merge") no longer holds: `feat/rooms-games-epic` merged in `3f75aa614`. See 5-01. |
| 2-08 game map ids hand-synced | minor | Open | `game_rooms/types.rs:11-18` (`GameKind::maps`), `ui/src/lib/api/game-room-types.ts:3` (`GameRoomMap`), `kart/tracks/defs.ts` and `shooter/catalog.ts`. `unit/roomGamesKartTracks.test.ts:25` pins the UI list to a literal, not to the server. Dead City (`deadcity`) was added in this window to three of the lists by hand. |
| 3-02 (residual) Atlassian `is_success()` | nit | **Fixed** | `confluence.rs` has 0 `is_success()`; `jira.rs` has 3, all after `self.t.send_raw(...)` (`:545`, `:618`) or on the best-effort dev-status fan-out that builds on `self.t.http()` (`:1090`), so TLS refusal and the client cache still apply. |
| 3-03 route files as the budget and transcript service layer | minor → **nit (residual)** | Fixed in substance | `otto-server/src/budget/mod.rs` (`BudgetGate`) and `transcript/resolve.rs` exist; `load_budgets_pub` / `budget_status_pub` are gone. **Residual:** `fold_opts` (`routes/transcript.rs:147`) and `register_work_artifact` (`:645`) are still defined in the route file and called by the live tailer (`transcript_tail.rs:368,771,825`); 9 callers still use the `routes::usage::` / `routes::transcript::` re-export paths (`swarm_host.rs:105`, `product_host.rs:480`, `budget_gate.rs:23`, `monitor.rs:454`, `assistant_host.rs:90`, `history_index.rs:165`, `agent_tasks_nudge.rs:85`). |
| 4-01 prompt-submit primitive in otto-review | major → **nit (residual)** | Fixed in substance | `otto-sessions/src/submit.rs` owns `bracketed_paste` (`:85`), `wait_for_tui` (`:124`), `dispatched` (`:163`) and `submit_prompt` (`:182`); `otto-review/src/session.rs:862` re-exports them; `scripts/check.sh:238-246` guards. **Residual:** the swarm runtime still assembles paste + Enter itself (`SwarmHost::{bracketed_paste, wait_for_tui, dispatched}`, `otto-swarm/src/runtime/host.rs:105-107`; `agent_run.rs:355-401` and `run.rs:832-876` repeat the same wait → submit → landed → re-inject block). It does take the submit lock (`engine.rs:112`). `swarm_host.rs:131-138` still reaches the primitives through the `review_session` alias. FIXES P08 parks this on a design call. |
| 4-02 three design-document assist engines | major | Partly fixed, still open | **Fixed (phase 1):** one `extract_fenced` in `otto-agent-run/src/fence.rs:8`; the Excalidraw split/merge in `otto-design-assist/src/excalidraw.rs:67,252`, used by canvas (`otto-canvas/src/assist.rs:42`); one `AgentTurn` (`otto-design-assist/src/ctx.rs:33`); `DesignFormat` reads `otto_design::format` (`otto-product/src/design_format.rs:6-9`). **Open:** three engines remain, each with its own busy registry (`otto-canvas/src/assist.rs:479` `SceneBusy`, `otto-design-assist/src/lib.rs:1027` `BusyGuard`, `mockup_assist.rs:252` `claim_mockup`) and live poll (`assist.rs:736`, `lib.rs:1676`, `mockup_assist.rs:439`). **The drift the finding predicted happened again in this window:** the backend-performance-4-05 `StatGate` fix reached the canvas and Design Hall polls only; `mockup_assist.rs:455-470` still re-reads the whole file every tick through `read_text_capped`. `db_assist.rs:809` kept its own `extract_fenced`. The scene3d model still lives in `otto-product/src/design_scene3d.rs` (validated via the `crate::design_scene3d` re-export at `mockup_assist.rs:216,468`). Phase 1 also added a heavy edge: `otto-canvas/Cargo.toml:11` now depends on `otto-design-assist` (which pulls `otto-improve`, `otto-memory`, `otto-sessions`, `otto-skills`) to reach `excalidraw.rs`, whose only import is `serde_json::Value` (`:9`). |
| 4-03 workflow triggers split | minor | **Fixed** | `otto_workflows::triggers::validate_spec` (called at `routes/workflows.rs:1613`), `FIREABLE_EVENT_KINDS` with a UI parity test, and the schedule tick in `otto-workflows/src/triggers/schedule.rs`; `workflow_trigger_scheduler.rs` is an 84-line supervisor. |
| 4-04 function ratchet only in otto-server | minor → **nit (residual)** | Fixed in substance | `loc-baseline.json` `function_max: 400` over every `crates/**/src` file, 23 named over-cap entries that can only shrink. **Residual:** `review_run::execute` (872, `workflow_engine/nodes/review_run.rs:7`) and `serve_terminal` (584, `otto-sessions/src/ws.rs:1641`) were not split (FIXES P02). |
| 4-05 fourth SQL classifier in otto-mcp | nit | **Fixed** | No prefix check left in `otto-mcp/src`; `ensure_read_only` in the DB viewer is the only gate. |

No earlier architecture finding was rejected.

## Areas covered in this iteration

- **Earlier findings.** Re-verified all 19 open findings from iteration 4 against the code, as above.
- **Changes since iteration 4.** Walked all 44 commits (`6cff08270..8177c4d85`): MCP profiles
  (`otto-mcp/src/profiles/`, `otto-state/src/mcp_profiles.rs`, ottod `mcp_tools/profile.rs`,
  `mcp_server.rs`, `otto-sessions/src/http.rs:541-562`, UI `ProfilesTab`, `McpProfilePicker`),
  the Jira create-issue feature (`otto-issues/src/{issue_create,md_adf,adf_check,markdown,md_storage}.rs`,
  `jira/create.rs`), the room games and the classroom.
- **MCP profiles.** A decorator (`ProfiledTools`, `profiles/mod.rs:505`) on the outward surface and
  thin hooks on the two ottod surfaces. The design (narrow-only, never a grant) is sound. The two
  ottod copies of "enabled path + parse filter" (`mcp_tools/profile.rs:21-41`,
  `mcp_server.rs:127-140`) differ only in id sanitising, and the daemon re-parses the ids, so there
  is no finding.
- **otto-issues Markdown pipeline.** One parser (`markdown.rs`) feeds both the ADF and the storage
  emitters, each with a validator. Well factored. (`adf::text_to_adf` is now used only by a test,
  which is the maintainability lens's call.)
- **MCP governance.** Traced both choke points end to end: `McpService::invoke`
  (`otto-mcp/src/service.rs:662`) and `governed_invoke` (`otto-server/src/mcp_outward.rs:207`).
  This produced 5-03.
- **Agent turn runners.** Traced every `agent_session::run_session_turn*` caller (16 sites) and the
  host traits that forward to it. This produced 5-02.
- **Room relays and games.** `rooms/` vs `game_rooms/` registry, socket and auth, and the UI
  `room-client.ts` vs `games/client.ts`. This produced 5-01. The games UI itself
  (`rooms/games/**`, 19,408 lines) is split by concern (sim, render, net, maps, modes), the largest
  file is 792 lines, and `catalog.ts` derives its menus from the track and map tables. No god-file
  finding.
- **Classroom.** `home/school/*` and `ClassroomsBox.svelte` (796 lines). Status comes from the shared
  `lib/status.ts` `sessionState` (`model.ts:23,359`); kick-out, detention and release go through
  the workspace store's one delete/archive path with injected effects (`actions.ts:1-13`). No
  finding beyond 5-05.
- **UI cross-module imports.** Rebuilt the exact matrix (every relative import resolved) for HEAD
  and for `6cff08270`. The only edge that grew is `home → mission-control/lib` (2 → 4, classroom
  components importing `relTime`). Every other edge is unchanged or smaller (`agents → vault` and
  `panels → vault` are gone). The iteration-4 list was an abridged view; the full matrix (about 45
  edges) is mostly page-embeds-component reuse. No finding.
- **`database/ResultsGrid.svelte` reuse by AWS** (iteration-4 open question). AWS Athena/Logs and
  the comparison view pass no `connectionId`, so `hosted` (`:177`) is false and every
  Explorer-coupled branch (filters `:515`, quick-filter menu `:940`, pending edits `:372`, pager
  `:271`) is gated; `autoInputs` (`:401`) is only consulted when the owner passes `onviewmode`.
  Coupled but guarded; no finding.
- **otto-design** `store.rs` (3,186) and `service.rs` (2,945): cohesive domain service over its own
  `design_*` tables and blob store. No finding.
- **otto-state** `swarm.rs` (2,724) and `product.rs` (2,930): plain CRUD and row mappers; the swarm
  acting-user rule lives in `otto-swarm/src/runtime/run.rs:252`. No finding.
- **otto-rbac** `tokens.rs` `authenticate` (277 lines, `:248`): one query plus per-kind checks; the
  persisted `kind` discriminator is used consistently in SQL. No finding.
- **product `OverviewTab.svelte`.** Not covered before. This produced 5-04.
- **Three.js stages.** `home/school/scene.ts`, `rooms/games/common/view.ts`,
  `design-hall/studio3d/embed.ts`. This produced 5-05.
- **Example plugins.** `examples/plugins/team-performance/server.js` is 3,138 lines with 86
  functions (scan, scope, overview, reports, coach, HTTP), but it is an example outside the product
  build, with its own `lib/` split (27 modules) and suites. Noted, not scored.
- **otto-server new files** (3 weeks): boot/, budget/, transcript/, host impls, workbench and session
  files routes. `routes/workbench.rs` is a thin layer over `otto_state::WorkbenchRepo`. No finding.

## Areas NOT covered yet (next iteration)

- **otto-usage** `engine.rs` (about 2,800 lines) internals.
- **otto-dbviewer drivers:** `mongodb.rs` (4,974 total, ratcheted at 4,224 production) and
  `clickhouse.rs` (3,693): what the drivers repeat that `sql_common.rs` (170) could hold.
- **Large functions:** `run_analysis` (580) / `run_generate_plan` (446), `controller_inner` (472),
  `bridge::handle` (451), `static_review` (585), `run_skill_eval_core` (446). All are ratcheted.
- **UI:** `git/ReviewPanel.svelte` (2,529, 52 commits in 3 weeks across the git panels),
  `git/DiffViewer.svelte` (2,344), `git/WipPanel.svelte` (1,606), `database/DatabasePage.svelte`
  (2,814), `QueryEditor.svelte` (2,230), `QueryBuilder.svelte` (2,177), `lib/stores/apiClient.svelte.ts`
  (2,308, 66 methods, 23 commits in 3 weeks), `usage/UsagePage.svelte` (2,062),
  `api/RequestBuilder.svelte` (1,968), `vault/DocsAgentsView.svelte` (1,802),
  `lib/components/FloatingBar.svelte` (1,581).
- **UI API clients.** Raw `api.get/post` calls are the norm (git 105, settings 94, workflows 33);
  whether repeated endpoints (`/repos/{}/refs` ×5, `/repos/{}/review-config` ×4) want typed clients
  like `lib/api/issues.ts` was not judged.

---

## New findings

### architecture-5-01: minor: the two room relays each hand-roll capability tokens, socket auth and rate limits, and the security hardening has already drifted

**Location:** `crates/otto-server/src/rooms/registry.rs:20`, `:35-56`, `:147-156`, `:305-319`; `crates/otto-server/src/rooms/auth.rs:31`; `crates/otto-server/src/game_rooms/registry.rs:14`, `:65-94`, `:208-214`, `:240-246`; `crates/otto-server/src/game_rooms/socket.rs:14-45`; `ui/src/modules/rooms/room-client.ts:40-89`; `ui/src/modules/rooms/games/client.ts:33-51`

**Evidence**

`rooms/` (session-sharing rooms) and `game_rooms/` (the multiplayer games merged in this window) are
two in-memory relays with the same capability model: a random 32-byte token, base64url-encoded,
stored only as a SHA-256 digest, presented over the `Sec-WebSocket-Protocol` header, plus a
token-bucket rate limit and a reconnect grace. Each writes all of it itself:

| Piece | `rooms/` | `game_rooms/` |
|-------|----------|---------------|
| token + digest | `secret()` / `hash()` `registry.rs:147-156` | `token()` / `hash()` `registry.rs:87-94` |
| token-bucket `Rate` | `registry.rs:35-56` | `registry.rs:65-86` (same body, different message) |
| reconnect grace | `GRACE = 30 s` `:20` | `GRACE = 20 s` `:14` |
| subprotocol token parse | `auth.rs:31` `socket_token` | `socket.rs:33` `socket_token` (copy, `"otto-game"`) |
| WS upgrade (`query` refusal, authenticate, size caps) | `socket.rs:18-36` | `socket.rs:14-31` |
| UI client (subprotocol token, backoff reconnect, heartbeat) | `room-client.ts:40-89` | `games/client.ts:33-51` |

The hardening already diverged. `rooms` compares the digest in constant time and says why
(`registry.rs:312-317`: "Accumulating XOR prevents an early-byte comparison from making a useful
token prefix oracle"). `game_rooms` compares with `==` on the array, both for the member credential
(`registry.rs:245`, `member.hash == hash(credential)`) and for the invite (`:211`,
`room.invite != Some(hash(invite))`).

**Cost**

These are the only unauthenticated-capability surfaces on the daemon besides share links. Every
change to the capability rules (comparison, token length, header parsing, rate limits) has to be
made twice, and the first divergence shipped in the same window the second relay landed. A third
relay (2-06 anticipates an `otto-rooms` crate) would copy them again.

**Fix**

1. Add `otto-server/src/rooms/capability.rs` (or `otto_core::capability` if 2-06's crate lands first)
   with `Capability::issue() -> (String, Digest)`, `Digest::matches(&str) -> bool` (the constant-time
   fold), `TokenBucket::take(rate, burst, msg)` and `socket_token(headers, protocol)`.
2. Make both registries and both upgrade handlers use them; delete the copies. The game invite and
   member checks then become constant-time by construction.
3. In the UI, extract a `RelaySocket` (subprotocol token, backoff, heartbeat/deadline) in
   `modules/rooms/relay-socket.ts` that `RoomClient` and `GameClient` wrap.
4. Do this as the first step of 2-06, so the crate move carries one primitive set.

### architecture-5-02: minor: the resumable session-turn runner lives in otto-server though it needs only `SessionManager`, so four host traits exist just to forward to it and assist engines stay in the server

**Location:** `crates/otto-server/src/agent_session.rs:1-24`, `:157`, `:190`, `:245`, `:706-708`; `crates/otto-server/src/agent_session_lifecycle.rs` (649 lines), `agent_session_reply.rs` (137); `crates/otto-server/src/automation_ctx.rs:145-165`; `crates/otto-server/src/product_host.rs:445-460`; `crates/otto-server/src/modules/ctx_impls.rs:552`, `:648`; `crates/otto-canvas/src/assist_ctx.rs:56`; `crates/otto-design-assist/src/ctx.rs:71`; `crates/otto-agent-run/Cargo.toml:15`

**Evidence**

`agent_session` (2,145 lines with its two `#[path]` files) runs one agent turn against a resumable
session: create or resume, submit, watch the transcript turn count grow, fail fast on provider
errors. Its own header says it "mirrors `otto_swarm::runtime::agent_run`" (`:7`). Its public
functions take `&ServerCtx` (`:157`, `:190`, `:245`), but the production body reads only
`ctx.manager` (45 uses; no other field). Everything else it calls is already outside otto-server
behind re-exports: `crate::turn_oracle` and `crate::offload` are `otto_agent_run::*`
(`lib.rs:58,104`), and `crate::review_session::submit_prompt` (`:707`) is
`otto_sessions::submit`. `require_owned_resume` (`:36`) needs only the pool. `otto-agent-run`
already depends on `otto-sessions` (`Cargo.toml:15`), so nothing blocks the move.

Because the runner is in the server, every domain crate that needs a turn grows a host-trait method
whose otto-server impl only forwards:

- `CanvasAssistCtx::run_agent_turn` (`otto-canvas/src/assist_ctx.rs:56`) →
  `ctx_impls.rs:552` `run_session_turn`;
- `DesignAssistCtx::run_agent_turn` (`otto-design-assist/src/ctx.rs:71`) →
  `ctx_impls.rs:648` `run_session_turn_with`;
- `AutomationCtx` → `automation_ctx.rs:152`;
- `ProductStudioHost::run_session_turn` (`otto-product/src/host.rs:120`) → `product_host.rs:453`.

And the engines that call it directly stay in otto-server: `mockup_assist.rs` (768 lines),
`db_assist.rs` (920), `modules/pr_draft.rs`, `vault_docs_agent.rs` (4,093, 4 call sites),
`modules/review/summarize.rs`, `routes/browser.rs:1061` and `workflow_engine.rs:4407`.

**Cost**

Each forwarding impl chooses the turn options, which is how canvas got idle-only completion
(`ctx_impls.rs:552`, `STUCK_IDLE`) while Design Hall got `done_file` and `kill_on_stall`
(`:648`), the 4-02 divergence. The runner is the missing piece that keeps 4-02 phase 2
(`AssistTarget`) and 1-04 (a shared fan-out runner in `otto-agent-run`) from being a pure domain
move: neither can live below otto-server while the turn runner does not. Each new agent-using
feature adds another forwarding method or another server-resident engine.

**Fix**

1. Move `agent_session{,_lifecycle,_reply}.rs` into `otto-agent-run/src/session_turn/`, taking
   `&Arc<SessionManager>` (and `&DbPool` for `require_owned_resume`) instead of `&ServerCtx`. Keep
   `pub use otto_agent_run::session_turn as agent_session` in otto-server for the 16 callers.
2. Drop the four forwarding methods: canvas, design-assist, automation and product call
   `otto_agent_run::session_turn::run_*` directly with the `SessionManager` their ctx already
   exposes (or one `fn sessions(&self) -> &Arc<SessionManager>` accessor).
3. Then move `mockup_assist` / `db_assist` into the 4-02 shared engine, and build 1-04's fan-out
   runner on the same crate.

### architecture-5-03: minor: MCP governance is two pipelines with disjoint rule systems, so each new governance feature lands on one side only

**Location:** `crates/otto-mcp/src/service.rs:662-805` (`McpService::invoke`, `policy::evaluate` at `:805`); `crates/otto-server/src/mcp_outward.rs:198-207`, `:440-461`, `:605-610`, `:765-770`; `crates/otto-server/src/mcp_auto_approve.rs`; `docs/features/mcp-control-plane.md:45`, `:528-534`

**Evidence**

`otto-mcp` describes itself (AGENTS.md crate table) as "the governance pipeline every governed tool
call funnels through", and the feature guide's summary table says "One pipeline
(`McpService::invoke`)" (`mcp-control-plane.md:45`). In fact there are two:

- `McpService::invoke` (`service.rs:662`) for registered upstream servers: server gate → allowlist →
  per-tool permission → **policy-as-code** (`policy::evaluate`, `:805`) → risk/approval → dry-run →
  execute → fail-closed audit.
- `governed_invoke` (`mcp_outward.rs:207`, 557 lines, ratcheted) for
  Otto's own `otto.*` tools: per-token scope → enable list → personal-agent policy → reference
  resolution → **auto-approve rules** (`:446`, `crate::mcp_auto_approve::resolve_for_call`) →
  approval → dry-run (`:605`) → self-call execute → audit, with its own `deny_audit` (`:765`).

They share small pieces (`otto_mcp::PendingAuditRow`, `canonical_hash`, `outcome_to_resp`), but the
rule systems are disjoint. `mcp_policies` never applies to `otto.*`, and the guide says so
(`:533-534`: "Allowlists, policies and per-tool server rules below do not apply to them"). Auto-approve
rules exist only on the `otto.*` side (`otto-mcp/src/service.rs` has no `auto_approve` reference).
MCP profiles, added this window, filter `otto.*` and the gateway but are applied by three surface
hooks rather than at either choke point.

**Cost**

An admin who writes a "require approval" or "deny" policy in the Rules drawer gets no effect on
`otto.create_pr` or `otto.merge_pr`, the tools agents use most. An auto-approve rule cannot cover a
registered server's tool. Every governance feature so far (policies, auto-approve, token scopes,
personal-agent policy, profiles) was built for one pipeline, and parity needs a second
implementation each time.

**Fix**

1. Extract the shared stages from `McpService::invoke` into an `otto_mcp::governance` module:
   `Stages { allowlist, permission, policy, auto_approve, approval, dry_run, audit }` over a
   `GovernedCall { server_id, tool, args, auth, ws }`.
2. Make `governed_invoke` a caller of it, with `server_id = "otto"` (a pseudo-server). Pre-stages
   (token scope, personal-agent policy, reference resolution) stay in otto-server as hooks, and the
   executor is the self-call. `mcp_policies` and auto-approve rules then apply to both sides.
3. Move auto-approve resolution next to `policy.rs` in otto-mcp, and fix the guide's "One pipeline"
   row to match.

### architecture-5-04: minor: product `OverviewTab.svelte` is a 2,997-line Jira issue editor, draft editor and attachment resolver in one component, with 31 commits in 3 weeks

**Location:** `ui/src/modules/product/OverviewTab.svelte:53`, `:77-104`, `:134-176`, `:178-209`, `:245-300`, `:386-457`, `:474-600`, `:766`; `ui/scripts/ui-guards-baseline.json:197`

**Evidence**

Not covered by earlier iterations. The file is the fourth-largest UI component, capped at its
current 2,997 lines by the ui-guards baseline (`:197`), and grew from 2,982 during this window (31
commits in 3 weeks, 48 in 6). Its own section banners show at least seven independent concerns, each
with its own state, loaders and stale-request guards:

- discovery-swarm launch (`:53`, `:357-385`);
- the live Jira issue: load (`:245-300`, `loadIssueFull` `:458`), dev-status (`:93`, `:474`),
  inline title and description editors that write to Jira (`:100-113`, `:504-600`);
- the Otto draft editor (`:134-152`, seeding rules `:319-356`);
- attachment-token resolution for the body preview, with its own object-URL cache, in-flight set
  and sequence guard (`:159-176`, `:386-457`), which "mirrors AttachmentsPanel.loadAttUrl guard"
  (`:166`) instead of sharing it;
- transcripts with a find provider (`:178`, `:766`);
- comments, the publish dialog, tags and related stories (`:185-244`).

Three separate object-URL lifecycles are revoked in four places (`:275`, `:282`, `:311-314`, `:601`).

**Cost**

Jira writes (live, outward-facing) and Otto draft edits (local) share one component's reactive
scope, so a change to either re-touches the other's guards. The file churns daily and is pinned at
its cap, so the next feature here either breaks the guard or forces an unplanned split.

**Fix**

1. Move the Jira half (issue load, dev-status, title and description editors) into
   `product/jira/JiraIssueSection.svelte`, beside the existing `JiraIssueEditor`, keeping the
   `onStory(s)` guard pattern.
2. Extract `product/attachmentUrls.svelte.ts` (an authed object-URL cache with an in-flight set,
   sequence guard and one `dispose()`), used by both the body preview and `AttachmentsPanel`.
3. Move the draft form into `DraftEditor.svelte` and transcripts into `StoryTranscripts.svelte`.
   Lower the baseline after each move.

### architecture-5-05: nit: three hand-rolled three.js stages each handle the WebGL lifecycle differently

**Location:** `ui/src/modules/home/school/scene.ts:24`, `:597`, `:611`; `ui/src/modules/rooms/games/common/view.ts:32`, `:149-163`; `ui/src/modules/design-hall/studio3d/embed.ts:165`, `:365`, `:445-450`

**Evidence**

The classroom, the room games and the Design Hall 3D studio each create and manage their own
`WebGLRenderer`. Each solved the lifecycle separately:

- **Classroom:** pauses on `visibilitychange` (`:597`) and remounts on `webglcontextlost`
  (`:611`, added this window).
- **Games:** `GameView` (`view.ts:32`) has neither handler; it only disposes and forces context loss
  on teardown (`:149-163`).
- **Studio:** `embed.ts` pauses off screen with an `IntersectionObserver` (`:365`) but has no
  context-loss recovery.

**Cost**

A GPU reset during a multiplayer game or a studio session leaves a black canvas, a fix the classroom
already has. Each new 3D surface (a fourth is likely, given the games roadmap) starts from scratch.

**Fix**

Add `lib/three/stage.ts`: a renderer with capped pixel ratio, an off-screen/hidden pause
(IntersectionObserver + `visibilitychange`), context-loss remount and a `dispose()` that frees
tracked meshes and calls `forceContextLoss()`. Make `GameView`, `mountSchool` and the studio embed
use it.

---

## Carried-over findings (still open; full text in `../iter-1/` to `../iter-4/architecture.md`)

| ID | Severity | Location | Remaining work |
|----|----------|----------|----------------|
| architecture-1-04 | major | `crates/otto-server/src/skill_review.rs:400`, `vault_docs_agent.rs:356`, `modules/review/summarize.rs` | Add a shared fan-out and summarizer runner in `otto-agent-run`, after 5-02. |
| architecture-1-09 | major | `ui/src/modules/git/GraphView.svelte:1` | 4,887 lines. Extract RefSidebar and the git action handlers. |
| architecture-1-10 | major | `ui/src/lib/stores/database.svelte.ts:125` | 3,661 lines. Split assist, connection warm/keep-alive, schema tree, saved queries and history into sub-stores behind the facade. |
| architecture-4-02 | major | `crates/otto-canvas/src/assist.rs:479,736`, `otto-design-assist/src/lib.rs:1027,1676`, `otto-server/src/mockup_assist.rs:252,439`, `db_assist.rs:809`, `otto-canvas/Cargo.toml:11` | Phase 2 (`AssistTarget`); meanwhile give `mockup_assist` the `StatGate`, delete `db_assist`'s `extract_fenced`, move `excalidraw.rs` to a crate canvas can depend on cheaply (`otto-design::format` or `otto-agent-run`), and move scene3d into otto-design. |
| architecture-1-01 (residual) | minor | `crates/otto-server/src/workflow_engine.rs:1850` | `run_workflow_with_budget` is 1,250 lines. Introduce `RunState`, then split admission and scheduling. |
| architecture-1-05 | minor | `crates/otto-mcp/src/outward/exec.rs:393`, `crates/ottod/src/mcp_tools.rs:2542`, `:3092` | Route native twins through `route_for` (strip the `/api/v1` prefix in one place) instead of `read_route` / `run_tool` arms. |
| architecture-1-07 (residual) | minor | `crates/otto-server/src/routes/api_client.rs:1808-2100`, `:86` | Move `execute`, `execute_service` and `resolve_environment` into `otto-apiclient::rest` behind a ctx trait; call `otto_netguard` directly. |
| architecture-1-08 (residual) | minor | `ui/src/modules/workflows/WorkflowsPage.svelte:285`, `:1003`, `:1445` | Move live run sync, the final-output panel and approvals into `RunMonitor.svelte`. |
| architecture-1-13 | minor | `ui/src/lib/uiCommands/k8s.ts`, `uiCommands/home.ts`, `stores/swarm.svelte.ts`, `stores/canvas.svelte.ts`, `router.svelte.ts` | 21 `lib` → `modules` imports left. |
| architecture-2-06 | minor | `crates/otto-server/src/rooms/`, `game_rooms/` | Unblocked (games merged). Move to an `otto-rooms` crate, starting with 5-01. |
| architecture-2-08 | minor | `crates/otto-server/src/game_rooms/types.rs:11` | One game catalog (served by the daemon or generated JSON) read by both sides. |
| architecture-2-01 (residual) | nit | `ui/src/lib/components/Terminal.svelte:1108`, `:1324` | Move the resize half into `TermGridSync`. |
| architecture-3-03 (residual) | nit | `crates/otto-server/src/routes/transcript.rs:147`, `:645` | Move `fold_opts` and `register_work_artifact` into `transcript/`, and point the 9 callers at `crate::budget` / `crate::transcript`. |
| architecture-4-01 (residual) | nit | `crates/otto-swarm/src/runtime/host.rs:105-107`, `agent_run.rs:355-401`, `run.rs:832-876` | Give `submit_prompt` a guard closure for the run-active check and convert swarm; delete the `review_session` alias use in `swarm_host.rs`. |
| architecture-4-04 (residual) | nit | `crates/otto-server/src/workflow_engine/nodes/review_run.rs:7`, `crates/otto-sessions/src/ws.rs:1641` | Split `review_run::execute` (872) and `serve_terminal` (584). |
