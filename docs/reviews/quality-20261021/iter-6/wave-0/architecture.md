# Architecture review: quality loop 2026-10-21, iteration 6 (wave 0)

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

The review was read-only. It used `git log`, `git show`, `git diff`, `rg`/`grep`, `sed`, `wc` and a
Python read of `scripts/loc-baseline.json` and `ui/scripts/ui-guards-baseline.json`. Nothing was
built or tested. Every finding was confirmed against `fix/quality-iter6-20261023` at `02a64019b`
(the iteration-5 merge, PR #121).

Excluded: `target/`, `node_modules/`, `docs/reviews/`. The room games and the classroom are in scope.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Earlier findings still open, fully or as a residual | 0 | 4 (1-04, 1-09, 1-10, 4-02) | 9 (1-01r, 1-05, 1-07r, 1-08r, 1-13, 2-06, 5-02, 5-03, 5-04) | 3 (4-01r, 4-04r, 5-05r) |
| New in iteration 6 | 0 | 1 (6-01) | 4 (6-02, 6-03, 6-04, 6-05) | 0 |
| **Total** | 0 | 5 | 13 | 3 |

**Score: 10.0 − 5 × 0.3 − 13 × 0.1 − 3 × 0.03 = 10.0 − 1.5 − 1.3 − 0.09 = 7.11 / 10**

The score fell from 7.55 to 7.11. Iteration 5 fixed four findings: 2-01r, 2-08, 3-03r and 5-01. It
also made real progress on 1-10 (−349 lines) and 4-02 (the interim steps), but neither is closed.
The drop comes from areas no earlier iteration covered. The largest is 6-01: five subsystems each
decide on their own where the agent CLIs keep their transcripts, and they already disagree in
production. Usage and History both miss sessions that ran under a provider account.

To reach 9+, by weight:

- **6-01** (one transcript-roots definition in `otto-transcript`). It is cheap and mechanical, it
  removes a production data gap, and it is the only new major.
- **The four carried majors.** 5-02 (move `agent_session` into `otto-agent-run`) still unlocks both
  1-04 and 4-02 phase 2. 1-09 and 1-10 need a package that can run `npm run check`.
- **6-02 together with 6-01.** Both belong in `otto-transcript`, which AGENTS.md calls "the one
  parser for the agent CLIs' on-disk transcripts".

## Status of earlier findings

| ID | Sev (then → now) | Status | Evidence now |
|----|------------------|--------|--------------|
| 1-01 (residual) `run_workflow_with_budget` | minor | Open | `workflow_engine.rs:1850`; `loc-baseline.json` `functions.run_workflow_with_budget: 1250`. Iteration 5 made `execute_node` an 18-line shim over `nodes::dispatch` (`execute_node: 35`), but there is no `RunState` and no admission/schedule split. |
| 1-04 fan-out + summarizer per feature | major | Open (deferred) | `skill_review.rs:400` (`agent.name == "summarizer"`), `vault_docs_agent.rs:14` ("Mirrors the `product_run` fan-out + summarizer"), `:356` (`SUM_RETRY_IDX = usize::MAX`), `modules/review/summarize.rs` (`summarize_and_persist` 467, ratcheted). `otto-agent-run/src` holds `agent_run, fence, live_file, offload, screen, turn_oracle` and no fan-out runner. Blocked on 5-02. |
| 1-05 two MCP surfaces | minor | Open | `otto.list_prs` is built at `otto-mcp/src/outward/exec.rs:626`. `otto_list_prs` is built again at `ottod/src/mcp_tools.rs:2635`, with its own `opt_query`. `route_for` 1,555, `run_tool` 1,189, `read_route` 446 and `base_tool_catalog` 687 are unchanged. FIXES P09: "deferred". |
| 1-07 (residual) REST engine in a route file | minor | Partly fixed, still open | **Fixed:** the `net_guard` re-export is gone; callers use `otto_netguard::` directly. **Open:** `routes/api_client.rs` (3,261 lines) still defines `execute` (`:1803`), `execute_service` (`:1821`) and `resolve_environment` (`:2012`). It is called from outside the route layer by `workflow_api_request.rs:50` and `routes/api_automation_runs.rs:103`. `otto-apiclient/src/rest.rs` holds only the pure pieces (curl, OpenAPI, json_path, assertions). |
| 1-08 (residual) `WorkflowsPage.svelte` | minor | Open | 3,710 lines, unchanged. `refetchRun` `:285`, `waitRunTerminal` `:1003` and the approvals are still in the page. |
| 1-09 `GraphView.svelte` | major | Open | 4,887 lines, capped by `ui-guards-baseline.json` `file-loc`. FIXES P02: "needs `npm run check`". |
| 1-10 `DatabaseStore` | major | Partly fixed, still open | 3,661 → 3,312 lines. Assist, saved queries and history moved to `lib/stores/database/{assist,savedQueries,history}.svelte.ts`. Tabs, connection warm/keep-alive, SSH tabs, the schema tree, object ops, query runs/plans and filters are still in the class. FIXES P04 says the connection and schema-tree halves are "tightly coupled to `connEpoch`". |
| 1-13 `lib/` imports `modules/` | minor | Partly fixed, still open | 21 → 19. Left: `router.svelte.ts:18-19` (room and game invites), `stores/canvas.svelte.ts:16`, `stores/product.svelte.ts:72`, `stores/apiClient.svelte.ts:26`, `stores/mockup-assist.svelte.ts:21`, `stores/swarm.svelte.ts:8,31`, `stores/k8s.svelte.ts:16`, `stores/assistant.svelte.ts:19`, `uiCommands/k8s.ts:22-23`, `uiCommands/browser.ts:17`, `uiCommands/home.ts:11-12`, `uiCommands/swarm.ts:12`, `uiCommands/vault.ts:13`, `uiCommands/brokers.ts:28`, `uiCommands/git.ts:19`. |
| 2-01 (residual) terminal grid negotiation | nit | **Fixed** | `termGridSync.ts` exports `TermGridSync`, `resizeSendStep` and `shouldDeferLocalResize` (`Terminal.svelte:223`). `deferLocalResize` (`:1105`) and `sendResize` (`:1321`) are thin call sites over those pure decisions, which are tested by `unit/terminalResize.test.ts`. |
| 2-06 rooms inside otto-server | minor | Open (deferred) | `otto-server/src/rooms/` (plus `recap/` and `recap_engines/`) and `game_rooms/` are still in the server. Step 1 (5-01) landed. |
| 2-08 game map ids hand-synced | minor | **Fixed** | `ui/src/modules/rooms/games/catalog.json` is read by `game_rooms/types.rs:13` (`include_str!`) and by `games/catalog.ts:9`. Tests: `catalog_lists_every_game_kind` and `unit/roomGamesCatalog.test.ts`. |
| 3-03 (residual) route files as service layer | nit | **Fixed** | `transcript/fold.rs` and `transcript/artifacts.rs` exist, and the route file no longer defines `fold_opts` or `register_work_artifact`. One stray `crate::routes::usage::trail_to_usage` call is left at `monitor.rs:454` (`routes/usage.rs:657`). It is too small to score; move it with the next `usage` touch. |
| 4-01 (residual) swarm paste + Enter | nit | Open | `otto-swarm/src/runtime/host.rs:105-107` (`bracketed_paste`, `wait_for_tui`, `dispatched`). `agent_run.rs:355-401` and `run.rs:854-898` still assemble wait → paste → re-inject by hand. The `review_session` alias in `swarm_host.rs` is gone. |
| 4-02 three design-document assist engines | major | Partly fixed, still open | **Fixed (interim):** `mockup_assist.rs:457` uses `otto_agent_run::live_file::StatGate`. `db_assist` has no own `extract_fenced` (only a test name at `:892`). Excalidraw lives in `otto_design::format`. `mockup_assist` claims through `otto_core::cancel_signal::InFlightGuard` (`:252`). **Open:** canvas (`otto-canvas/src/assist.rs:479` `SceneBusy`) and Design Hall (`otto-design-assist/src/lib.rs:1029` `BusyGuard`, `:1041` `try_busy`) still hand-roll the busy registry that `cancel_signal` provides. The three engines and their live polls remain. `otto-canvas/Cargo.toml:11` still depends on `otto-design-assist`, which pulls in `otto-improve`, `otto-memory`, `otto-sessions` and `otto-skills`. It does so only for `AgentTurn` (`assist_ctx.rs:30`) and a re-export path (`assist.rs:42` `otto_design_assist::excalidraw`, which is `pub use otto_design::format::excalidraw` at `lib.rs:86`). scene3d is still in `otto-product/src/design_scene3d.rs` (1,399 lines), with a 4-line otto-server re-export. |
| 4-04 (residual) over-cap functions | nit | Partly fixed, still open | **Fixed:** `serve_terminal` (a `TerminalConn` per frame type) is under the cap and no longer in `functions`. **Open:** `workflow_engine/nodes/review_run.rs::execute` is 615 lines (was 872). |
| 5-01 two relays hand-roll capability tokens | minor | **Fixed** | `rooms/capability.rs` (`Capability`, constant-time `Digest`, `TokenBucket`, `socket_token`) is used by both `rooms/registry.rs:3` and `game_rooms/registry.rs:3`, and by `game_rooms/socket.rs:5` and `rooms/auth.rs:34`. UI: `lib/relaySocket.ts` sits under both `room-client.ts` and `games/client.ts`. |
| 5-02 turn runner in otto-server | minor | Open (deferred) | `otto-server/src/agent_session.rs` (1,359), `_lifecycle.rs` (649) and `_reply.rs` (137) are unchanged. FIXES P09: "deferred to the next iteration". |
| 5-03 two MCP governance pipelines | minor | Open (deferred, needs a decision) | `policy::evaluate` is only in `otto-mcp/src/service.rs:841,1106`. Auto-approve is only in `mcp_outward.rs:446`. `docs/features/mcp-control-plane.md:45` still says "One pipeline". |
| 5-04 product `OverviewTab.svelte` | minor | Partly fixed, still open | **Fixed:** `product/attachmentUrls.svelte.ts` is shared with AttachmentsPanel (`OverviewTab.svelte:27`). **Open:** 2,971 lines. The Jira issue load and dev-status (`:89`, `:95`, `:253-286`), the draft editor and the transcripts (`:175`, `:312-322`) are still in the one component. |
| 5-05 (residual) three.js stages | nit | Partly fixed, still open | **Fixed:** `lib/three/stage.ts` is used by the games (`common/view.ts`) and the school (`school/scene/stage.ts`). **Open:** `design-hall/studio3d/embed.ts:28` imports only `cappedPixelRatio` and `releaseRenderer`. There is no `onContextLost` remount, and `watchVisibility` is not adopted (its own `IntersectionObserver` is at `:366`). |

No earlier architecture finding was rejected.

## Areas covered in this iteration

- **Earlier findings.** Re-verified all 20 open iteration-5 findings against `02a64019b`, as above.
- **Recent churn.** Ranked every file by commits in the last 3 weeks and checked the top 50: `types.ts`
  117, `otto-sessions/manager.rs` 51, `otto-server/modules.rs` 48, `policy.rs` 42,
  `workflow_engine.rs` 41, `otto-git/local.rs` 40, `ottod/mcp_tools.rs` 33, `workspace.svelte.ts`
  33, and so on. Also checked growth over 3 and 6 weeks for the large UI files.
- **Transcript discovery and parsing across crates.** `otto-usage/src/tailer/driver.rs`,
  `otto-server/src/{transcript/resolve.rs, history_index.rs}`, `otto-improve/src/evidence.rs`,
  `otto-sessions/src/{manager.rs, lifecycle.rs, accounts.rs}`, `otto-agent-run/src/turn_oracle.rs`
  and `ottod/src/main.rs::start_usage_tailer`. This produced 6-01 and 6-02.
- **otto-sessions `manager.rs`** (6,632 lines, ratcheted, 7,648 three weeks ago). Outlined every
  function. Provider-specific transcript code is mixed into the session manager; that is the 6-02
  half. The rest (spawn, attach, authority, sweep and suspend) is the manager's own job. No further
  finding.
- **otto-dbviewer drivers** (not covered before). `mysql.rs`, `postgres.rs`, `clickhouse.rs`,
  `mongodb.rs` and `redis.rs` share `sql_class::ReadClass` (`is_read_statement` is a one-line
  delegate in mysql and postgres), `sql_common::{assemble_bulk_graph, acquire_error,
  float_to_json}`, and the `complete::sql::assemble` completion cache through thin trait methods.
  What each driver keeps is wire-specific (cell decoders, cancel, plans). No finding.
- **UI stores and large files not covered before.** `lib/stores/apiClient.svelte.ts` produced 6-03.
  The `/repos/{id}/…` call sites across the git module produced 6-04. The windowing primitives in
  `lib/`, `git/`, `kubernetes/`, `database/` and `panels/` produced 6-05. `git/DiffViewer.svelte`
  (1,518 → 2,337 in 6 weeks) was outlined: its variable-height window is part of 6-05, and the rest
  (nav tree, search, PR composer, hunk staging) is diff-view work. No separate finding.
- **`ui/src/lib/api/types.ts`** (117 commits). It is already a façade over `lib/api/types/*.ts` and
  per-feature type files, and it churns because it mirrors the contract. No finding.
- **`otto-server/src/modules.rs`** (1,089 lines, 48 commits). It is a router-assembly seam whose
  children (`review/`, `session_input.rs`, `ctx_impls.rs`, `pr_draft.rs`) are already split. Review
  engine pieces come from `otto_review::engine`. Covered by 5-02 and 1-04. No new finding.
- **`policy.rs`** (2,694 → 3,449 in 3 weeks). This is the deliberately central route → feature table,
  fail-closed and coverage-tested, as decided in iterations 2 and 4. Its growth tracks new routes.
  No finding.
- **`ottod/src/main.rs`** (1,124). Process setup, listeners and teardown. The composition is in
  `otto-server/src/boot/`. Only the usage-tailer wiring was relevant (6-01).
- **New domain files in the window.** `otto-automation/scheduled_tasks_engine.rs`,
  `otto-insights/lib.rs`, `otto-apiclient/{rest,grpc}.rs`, `otto-pty/holder.rs` and
  `otto-issues/linear.rs` are in their domain crates, behind their ctx traits. No finding.

## Areas NOT covered yet (next iteration)

- **otto-usage `engine.rs`** (3,310) and **`clickhouse.rs`** (1,274) internals. Only the tailer's
  root discovery was read.
- **otto-dbviewer `service.rs`** (4,143) and **`types.rs`** (2,848): the split between the service
  façade and `service/*`.
- **otto-git `local.rs`** (5,695, 40 commits, was 6,648 three weeks ago): internal cohesion.
- **Large ratcheted functions:** `run_analysis` (580) / `run_generate_plan` (446),
  `controller_inner` (472), `bridge::handle` (451), `static_review` (585), `run_skill_eval_core`
  (446), `route_result_inner` (416).
- **UI:** `git/ReviewPanel.svelte` (2,519), `database/DatabasePage.svelte` (2,814),
  `QueryEditor.svelte` (2,230), `QueryBuilder.svelte` (2,200), `usage/UsagePage.svelte` (2,060),
  `api/RequestBuilder.svelte` (1,967), `vault/DocsAgentsView.svelte` (1,802),
  `lib/components/FloatingBar.svelte` (1,581), `agents/SessionView.svelte` (965 → 1,553 in 6 weeks).
- **Repeated typed-client gaps outside git:** settings (about 94 raw calls) and workflows (about 33).
  6-04 covers git only.
- **Hand-rolled HTML5 drag-to-reorder.** 12 components set `effectAllowed = 'move'`, and Navigator
  alone has three copies (`:235-254`, `:574-606`, `:699-724`). This was not judged for cost.

---

## New findings

### architecture-6-01: major: five subsystems each decide where agent transcripts live, and they already disagree in production

**Location:** `crates/otto-usage/src/tailer/driver.rs:213`, `:260-262`, `:1332-1352`, `:1386-1387`; `crates/ottod/src/main.rs:524`, `:535`; `crates/otto-server/src/transcript/resolve.rs:37-57`, `:126-136`, `:173-175`; `crates/otto-server/src/history_index.rs:1-5`, `:165`; `crates/otto-improve/src/evidence.rs:123-170`; `crates/otto-sessions/src/manager.rs:417-424`, `:1962-1966`; `crates/otto-sessions/src/lifecycle.rs:139-150`

**Evidence**

"Which directories hold Claude and Codex transcripts" is answered separately by each consumer:

| Consumer | Claude roots | Codex roots | Provider accounts | Isolated (e2e) mode |
|----------|--------------|-------------|-------------------|---------------------|
| Usage tailer (`driver.rs:1332`, `:1386`, built at `ottod/main.rs:529`) | `~/.claude/projects`, `~/.claude-*/projects`, `~/.config/claude/projects`, `<data>/provider-accounts/*/projects` | `~/.codex/sessions` only (`:261`, `:1387`), with no `$CODEX_HOME` | Claude yes, **Codex no** | `<data>/fixture-home`, gated on `OTTO_E2E == "1"` (`main.rs:524`, `:535`) |
| History index (`history_index.rs:165` → `resolve.rs:37`) | `~/.claude/projects` or `OTTO_TRANSCRIPT_ROOTS` | `$CODEX_HOME/sessions` | **none** | `<data>/e2e-transcripts/{claude,codex}`, gated on `"1" \| "true"` (`resolve.rs:174`) |
| Per-session resolve (`resolve.rs:126-136`) | account home, else `transcript_roots` | account home, else `transcript_roots` | both | as above |
| Improvement evidence (`evidence.rs:123-170`) | a hand copy of `resolve_transcript_sync`'s precedence | same | both | `<data>/e2e-transcripts`, gated on `== Ok("1")` only (`:158`) |
| Session manager (`manager.rs:417`, `:1962`) | (n/a) | account home, else `$CODEX_HOME/sessions` | Codex yes | none |

Provider accounts are real for both CLIs. `otto-sessions/src/accounts.rs:60-63` sets
`CLAUDE_CONFIG_DIR` or `CODEX_HOME` to the account home, and `accounts.rs:35` gives `codex login`.

Consequences visible in the code today:

- Codex sessions that run under a provider account write rollouts to
  `<data>/provider-accounts/<id>/sessions`. The usage tailer never lists that directory
  (`list_codex_files(&home)` at `driver.rs:752`), so their tokens are never imported. A user with
  `$CODEX_HOME` set is also invisible to Usage. The session manager, by contrast, honours both.
- The History index scans only the default roots (`history_index.rs:165`). Sessions run under any
  provider account, Claude or Codex, are missing from History search, although Usage counts the
  Claude ones.
- The isolated-daemon guard has two fixture locations (`fixture-home` and `e2e-transcripts`) and two
  predicates. The `OTTO_E2E` check is copied at 17 production sites. Fourteen accept `"1" | "true"`
  (`resolve.rs:174`, `workflow_engine.rs:276`, `run_engine.rs:131`, `finding_agent.rs:167`,
  `routes/findings.rs:995`, `otto-sessions/src/trust.rs:156`, `otto-orchestrator/src/lib.rs:147,166,196`, `agent_session.rs:232,328`, …). Three accept only `"1"`
  (`evidence.rs:158`, `ottod/main.rs:524`, `:535`). With `OTTO_E2E=true`, the tailer and
  improvement evidence would read the developer's real `~/.claude` while the rest of the daemon
  stays isolated. That is the leak AGENTS.md and the comment at `main.rs:522-523` say must not
  happen. The harness uses `1` today, so this drift is latent.

**Cost**

Every new provider root (a new account kind, another Claude config dir, agy or grok accounts) has to
be added in up to five places. The last two additions (accounts, `~/.claude-*`) each reached only
some of them, and users see the result as missing Usage and History data. The e2e isolation
guarantee rests on 17 copies of a predicate that has already forked.

**Fix**

1. Add `otto_transcript::roots` (the leaf crate that every consumer here already depends on, directly
   or through otto-sessions):
   - `pub fn isolated() -> bool` (the one `OTTO_E2E` predicate);
   - `pub struct TranscriptRoots { claude: Vec<(PathBuf, Label)>, codex: Vec<PathBuf> }`, with
     `TranscriptRoots::discover(home, data_dir)`. Precedence: `OTTO_TRANSCRIPT_ROOTS` → isolated
     fixture dir → defaults (`~/.claude/projects`, `~/.claude-*`, `~/.config/claude`,
     `$CODEX_HOME/sessions` else `~/.codex/sessions`) plus every
     `provider-accounts/*/{projects,sessions}`;
   - `TranscriptRoots::for_session(meta, data_dir)` (the account home, else the defaults).
2. Make the usage tailer, `history_index`, `transcript::resolve`, `otto-improve::evidence` and
   `SessionManager::codex_root_for` call it. Delete `claude_source_roots`, `list_codex_files`'s
   hard-coded root, `transcript_roots`, the evidence copy and `codex_sessions_root`. Replace the 17
   `OTTO_E2E` checks with `otto_transcript::roots::isolated()`.
3. Pick one fixture layout (`e2e-transcripts/{claude,codex}`) for the tailer too.
4. Add a test per consumer that a Codex rollout under `provider-accounts/<id>/sessions` and under
   `$CODEX_HOME` is found. Add a `check.sh` ratchet that forbids a new `env::var("OTTO_E2E")` outside
   `otto-transcript`.

### architecture-6-02: minor: the session manager has its own Claude/Codex "turn still open" parser beside the turn oracle, and their notification rules differ

**Location:** `crates/otto-sessions/src/manager.rs:1190`, `:1231-1320`, `:1344`; `crates/otto-agent-run/src/turn_oracle.rs:1-10`, `:212-345`, `:360`, `:636`; `crates/otto-agent-run/Cargo.toml:15`

**Evidence**

The idle sweep decides whether an agent is mid-turn with `agent_turn_open` → `claude_turn_open` /
`codex_turn_open` (`manager.rs:1231-1320`, about 90 lines over the raw JSONL). The turn oracle in
`otto-agent-run` folds the same transcripts for the same question: `ClaudeScanState`
(`turn_oracle.rs:225-345`) tracks `tail_end_turn`, sidechain skipping and notifications, and
`scan_codex` is at `:636`. The manager's own comment points at the copy ("same rule as the oracle's
scan").

The two rule sets have diverged:

- The manager re-opens a turn on any non-`attachment` line whose raw text contains
  `"<task-notification>"` (`manager.rs:1190`, `:1282`).
- The oracle parses notifications structurally (`find_task_notifications`, `:360`), with
  queue-operation-only tails handled separately (`:29`, `:66`, `post_end_turn_queue_only`).

The manager cannot call the oracle: `otto-agent-run` depends on `otto-sessions`
(`Cargo.toml:15`), not the other way round. Meanwhile `otto-transcript`, which both depend on, is
documented as "the one parser for the agent CLIs' on-disk transcripts" and owns neither rule.

**Cost**

Claude's transcript format changes often: sub-agent files, attachment stapling and queue
operations each needed a rule. Each change now has to be made twice, in two crates, and a miss
shows up in two different ways. The sweep either reaps a working agent or pins an idle one, which is
the fd leak `REAP_UNRESUMABLE_GRACE` exists to close (`manager.rs:1336-1343`). Meanwhile workflow
steps finish early or late.

**Fix**

Move the pure folds (`ClaudeScanState`/`ClaudeTail`, `scan_codex`/`CodexTail`, notification parsing)
from `turn_oracle.rs` into `otto_transcript::turn`. Keep the clocks and verdict logic in
`otto-agent-run`. Give the fold a `tail_open() -> Option<bool>` accessor and make
`SessionManager::agent_turn_open` call it, then delete `claude_turn_open`, `codex_turn_open` and the
manager's `TASK_NOTIFICATION`. Point the existing manager tests (for example the attachment-tail
case) at the shared fold. Do this alongside 6-01, since both live in `otto-transcript`.

### architecture-6-03: minor: `apiClient.svelte.ts` is a 2,308-line god store that nearly doubled in six weeks

**Location:** `ui/src/lib/stores/apiClient.svelte.ts:216-541`, `:542-682`, `:873-981`, `:982-1031`, `:1032-1294`, `:1440-1685`, `:1686-1789`, `:1808-2008`, `:2009-2266`; `ui/scripts/ui-guards-baseline.json` (`file-loc` 2,308)

**Evidence**

The store went from 1,232 lines six weeks ago to 1,548 three weeks ago and 2,308 now (22 commits in
3 weeks). It is pinned at its current size by the ui-guards `file-loc` baseline. One
`ApiClientStore` class holds 66 methods across independent concerns, each with its own state and
loaders, as its own section banners show:

- open-tab persistence and per-tab response slots (`:216-682`);
- loading, the storage gauge and change notifications (`:683-981`);
- the cookie jar (`:982-1031`);
- collections, including file import, Postman sync and git pull/push of collections (`:1032-1294`);
- requests and environments (`:1295-1439`);
- execution: HTTP, gRPC and GraphQL introspection (`:1440-1685`);
- curl import/export (`:1686-1789`);
- automations: the collection runner, its runs, progress and cancel (`:1808-2008`);
- draft helpers and history-to-draft (`:2009-2266`).

It also carries one of the 19 `lib → modules` imports (`:26`, `modules/api/graphqlVars`).
`DatabaseStore` (1-10) is the same shape and is being split into `lib/stores/database/*` behind an
unchanged façade.

**Cost**

The automations runner and the Postman/git sync are separate features that change on their own
schedule, but they share one reactive class with tab and execution state. Every feature that lands
here must either raise the guard or force an unplanned split. This store is on the same path the
DatabaseStore took to 4,400 lines.

**Fix**

Follow the 1-10 pattern: create `lib/stores/apiClient/{tabs,automations,collectionsSync,cookies}.svelte.ts`
as sub-stores. The `apiClient` façade delegates to them, and its public API stays unchanged. Start
with automations (`:1808-2008`, self-contained run state) and collections sync (`:1059-1265`). Move
`graphqlVars` into `lib/api/`. Lower the `file-loc` baseline after each move.

### architecture-6-04: minor: git endpoints have no typed client, so push, checkout and refs are issued from three layers with different follow-ups

**Location:** `ui/src/lib/stores/git.svelte.ts:405`, `:453`, `:510`, `:802`, `:816-821`; `ui/src/modules/git/pushFlow.ts:56-58`; `ui/src/modules/git/CreatePr.svelte:251`; `ui/src/lib/uiCommands/git.ts:299`, `:330`; `ui/src/modules/git/GitToolbar.svelte:171`; `ui/src/modules/git/GraphView.svelte:721`, `:864`, `:955`, `:971`, `:1837`, `:1848`; `ui/src/modules/git/LocalReviewPanel.svelte:168`

**Evidence**

There is no `lib/api/git.ts` (compare `lib/api/issues.ts` and `k8s.ts`). 21 files call
`api.get/post(`/repos/${id}/…`)` directly: `/prs/` ×14, `/refs` ×5, `/status` ×4, `/push` ×3,
`/checkout` ×3, `/diff` ×3, `/conflict/resolve` ×3, and others. What each call site does after a
mutation is decided locally, and the variants already differ:

- **Push.** `git.push()` (`git.svelte.ts:816`) stores the status and bumps `refsRev`. The agent
  command (`uiCommands/git.ts:330`) uses it but gets no rejected-push handling. The toolbar's
  `runPush` (`pushFlow.ts:58`) has the reject → Pull / force-with-lease flow and relies on its
  caller's `onstatus`. `CreatePr.svelte:251` posts `/push` raw, with neither the status update nor
  the `refsRev` bump, so an open graph and toolbar keep the stale ahead/behind until the next poll.
- **refsRev invalidation** is bumped by hand in three places (`git.svelte.ts:804`, `:819`,
  `uiCommands/git.ts:299`), not by the mutations themselves.
- **Refs.** `GraphView` fetches `/refs` four times (`:721`, `:864`, `:955`, `:971`), with three
  different error policies (record `miss`, record `miss`, swallow). `LocalReviewPanel.svelte:168`
  fetches it a fifth way.
- **Checkout** is posted from `GitToolbar.svelte:171` and from `GraphView.svelte:1837` and `:1848`.

**Cost**

Each new git action gets its post-mutation sync (status, refsRev, toasts, rejection handling)
reinvented at the call site. The divergence above is the result: two of the three push paths skip
pieces the third has. This also blocks 1-09: GraphView's action handlers cannot move out while each
one hand-rolls its request and refresh.

**Fix**

1. Add `lib/api/git.ts` with typed functions for the repeated endpoints (`refs`, `status`, `push`,
   `pushTarget`, `checkout`, `stage`, `prs.*`).
2. Put the post-mutation contract in the store: `git.push(repoId, opts)`, `git.checkout(...)` and
   friends always `setStatus` and bump `refsRev`. `pushFlow.runPush` becomes the UX wrapper over
   `git.push` (the rejection dialog), and the agent command and CreatePr call the same path. Add a
   `git.refs(repoId)` loader with one error policy.
3. Use it as the first step of the 1-09 GraphView split.

### architecture-6-05: minor: list windowing is implemented eight times, and the shared module-level helper lives in `git/` and lacks find-in-page

**Location:** `ui/src/lib/components/VirtualList.svelte:1-30`; `ui/src/lib/tableWindow.svelte.ts:1-120`; `ui/src/modules/git/list-window.svelte.ts:1-140`, `ui/src/modules/git/list-range.ts`; `ui/src/modules/kubernetes/monitor/tableWindow.ts:1-61`; `ui/src/modules/database/GridView.svelte:78-110`, `:197`; `ui/src/modules/database/SchemaTree.svelte:233`, `:277`, `:313-314`; `ui/src/modules/git/GraphView.svelte:464-469`, `:2111-2112`; `ui/src/modules/git/DiffViewer.svelte:51`, `:500-770`; `ui/src/modules/panels/FileTree.svelte:410`

**Evidence**

Eight pieces of code compute "rows in view + overscan + two spacers":

| Implementation | Where | Notes |
|----------------|-------|-------|
| `VirtualList` | `lib/components` | own scroller, uniform rows, ⌘F provider |
| `TableWindow` | `lib/tableWindow.svelte.ts` | table rows, measured pitch, `min`, ⌘F provider (`findRows`) |
| `ListWindow` + `listRange` | `modules/git/list-window.svelte.ts`, `list-range.ts` | outer scroller, measured pitch, `min`; **no ⌘F provider** |
| k8s `tableWindow.ts` | `modules/kubernetes/monitor` | table rows plus one expanded detail row |
| GridView | `database/GridView.svelte:78-110` | 2-D row and column window, its own ⌘F |
| SchemaTree | `database/SchemaTree.svelte:277-314` | tree rows |
| GraphView | `git/GraphView.svelte:2111` | fixed pitch; the comment at `:464` records an 18 px-per-row spacer drift fixed locally |
| DiffViewer | `git/DiffViewer.svelte:500-770` + `diff-virtual.ts` | variable height, measured, scroll anchoring |

`TableWindow.range` (`tableWindow.svelte.ts:56-62`) and `listRange` compute the same thing. Both
measure the pitch from a rendered row with the same `h > 4 && |Δ| > 0.25` rule. They differ only in
whether the list is its own scroller.

The feature drift is concrete. `TableWindow` and `VirtualList` register a find-in-page provider, so
⌘F searches unmounted rows. `ListWindow` does not, and none of its users (`WipPanel.svelte:563-564`,
`panels/FileTree.svelte:410`, `BlamePanel`, the DiffViewer file nav at `:368`) register one. Past
`min` rows (300 by default, 200 in DiffViewer's nav), ⌘F in a large WIP list, file tree or blame
finds only the mounted slice.

`ListWindow` is a generic primitive, but it lives inside the git module, and `panels/FileTree.svelte`
reaches into `../git/list-window.svelte` for it.

**Cost**

Every windowing fix (pitch measurement, phone layout, find support, scroll anchoring) has to be made
per copy, and the find gap shows one that was not. Each new long list picks whichever copy it finds
first.

**Fix**

1. Move `ListWindow` and `listRange` to `lib/listWindow.svelte.ts`, and make `TableWindow` the same
   class with a `selfScroll` option (it then shares `range` and `measure`). Give it the `findRows`
   provider, and wire it in WipPanel, FileTree, BlamePanel and the DiffViewer nav.
2. Re-express the k8s `tableWindow.ts` and the SchemaTree window over the shared `range`, with an
   "extra height at index i" hook for the expanded row.
3. Leave GridView (2-D) and DiffViewer (variable height) as the documented exceptions, but import
   the shared spacer and pitch helpers. Note the rule in `docs/design/guidelines`: new long lists use
   `VirtualList` or `listWindow`.

---

## Carried-over findings (still open; full text in `../../iter-1/` to `../../iter-5/architecture.md`)

| ID | Severity | Location | Remaining work |
|----|----------|----------|----------------|
| architecture-1-04 | major | `crates/otto-server/src/skill_review.rs:400`, `vault_docs_agent.rs:356`, `modules/review/summarize.rs` | A shared fan-out and summarizer runner in `otto-agent-run`, after 5-02. |
| architecture-1-09 | major | `ui/src/modules/git/GraphView.svelte` | 4,887 lines. Extract RefSidebar and the git action handlers, starting with 6-04's typed client. |
| architecture-1-10 | major | `ui/src/lib/stores/database.svelte.ts` | 3,312 lines. Split connections/warm-keep-alive and the schema tree (with `connEpoch` as a shared token), then object ops, query runs and filters. |
| architecture-4-02 | major | `crates/otto-canvas/src/assist.rs:479`, `otto-design-assist/src/lib.rs:1029-1051`, `otto-server/src/mockup_assist.rs`, `otto-canvas/Cargo.toml:11`, `otto-product/src/design_scene3d.rs` | Phase 2 (`AssistTarget`). Meanwhile, replace `SceneBusy` and `BusyGuard` with `otto_core::cancel_signal`'s `InFlightGuard`, move `AgentTurn` to `otto-agent-run` so canvas can drop its `otto-design-assist` edge, and move scene3d into `otto-design::format`. |
| architecture-1-01 (residual) | minor | `crates/otto-server/src/workflow_engine.rs:1850` | `run_workflow_with_budget` 1,250 lines. Add a loop-control enum, then `RunState`, then the admission/schedule split. |
| architecture-1-05 | minor | `crates/otto-mcp/src/outward/exec.rs:626`, `crates/ottod/src/mcp_tools.rs:2635` | Route the native twins through `route_for`. |
| architecture-1-07 (residual) | minor | `crates/otto-server/src/routes/api_client.rs:1803-2100` | Move `execute`, `execute_service` and `resolve_environment` into `otto-apiclient` behind a ctx trait. |
| architecture-1-08 (residual) | minor | `ui/src/modules/workflows/WorkflowsPage.svelte:285`, `:1003` | Move the live run sync, final output and approvals into `RunMonitor.svelte`. |
| architecture-1-13 | minor | 19 imports, listed above | Move the room and game invite capture to `lib/rooms`, and the swarm, product and assistant types to `lib/`. |
| architecture-2-06 | minor | `crates/otto-server/src/rooms/`, `game_rooms/` | An `otto-rooms` crate. Step 1 (5-01) is done. |
| architecture-5-02 | minor | `crates/otto-server/src/agent_session*.rs` | Move into `otto-agent-run::session_turn` and drop the four forwarding host methods. |
| architecture-5-03 | minor | `crates/otto-mcp/src/service.rs:841`, `crates/otto-server/src/mcp_outward.rs:446` | Needs a product/security decision; at minimum, fix the "One pipeline" row in `mcp-control-plane.md:45`. |
| architecture-5-04 | minor | `ui/src/modules/product/OverviewTab.svelte` | 2,971 lines. Extract JiraIssueSection, DraftEditor and StoryTranscripts. |
| architecture-4-01 (residual) | nit | `crates/otto-swarm/src/runtime/host.rs:105-107`, `agent_run.rs:355-401`, `run.rs:854-898` | Give `submit_prompt` a run-active guard closure and convert the swarm. |
| architecture-4-04 (residual) | nit | `crates/otto-server/src/workflow_engine/nodes/review_run.rs` | `execute` is 615 lines. Add a per-target context struct for the launch and score phases. |
| architecture-5-05 (residual) | nit | `ui/src/modules/design-hall/studio3d/embed.ts:28`, `:366` | Adopt `onContextLost` and `watchVisibility` from `lib/three/stage.ts`. |
