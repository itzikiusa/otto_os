# Architecture review: quality loop 2026-10-21, iteration 6 (wave 1)

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

This is a re-score after fix wave 1. It was read-only and used `git diff origin/main...HEAD`, `git
show`, `rg`/`grep`, `sed`, `wc`, `awk` and a Python read of `scripts/loc-baseline.json` and
`ui/scripts/ui-guards-baseline.json`. Nothing was built or tested. Every status was confirmed against
`fix/quality-iter6-20261023` at `37738d908`.

Excluded: `target/`, `node_modules/`, `docs/reviews/`. The scope is the same as the wave-0 report:
the same areas at the same depth, plus the wave-1 diff read for this lens.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Earlier findings still open, fully or as a residual | 0 | 4 (1-04, 1-09, 1-10, 4-02) | 9 (1-05, 1-07r, 1-08r, 1-13, 2-06, 4-01r, 5-03, 5-04, 6-03) | 2 (5-02r, 6-05r) |
| New in wave 1 | 0 | 0 | 0 | 2 (6w1-01, 6w1-02) |
| **Total** | 0 | 4 | 9 | 4 |

**Score: 10.0 − 4 × 0.3 − 9 × 0.1 − 4 × 0.03 = 10.0 − 1.2 − 0.9 − 0.12 = 7.78 / 10**

The score went from 7.11 to 7.78. Wave 1 fully fixed seven findings: 1-01r, 4-04r, 5-05r, 6-01
(the only new major), 6-02, 6-04 and 1-13's router part. It moved four more down to a smaller
residual or a lower severity: 5-02 (the turn runner is now in `otto-agent-run`), 6-05 (one shared
`ListWindow`), 1-13 (19 → 5 imports) and 4-02's interim steps. Two severity changes go the other
way:

- **4-01r went from nit to minor.** The wave added a public `RunGuard` / `submit_prompt_while` API.
  It has no production caller, and its synchronous `Fn() -> bool` cannot express the swarm's
  asynchronous DB run check, which is the caller it was built for.
- **6-05r went from minor to nit.** Only the k8s copy and two dead shims are left.

To reach 9+, by weight:

- **The four majors.** 1-04 needs the vault-docs and code-review ports onto `fanout::run`. 1-09 and
  1-10 need GraphView's RefSidebar and actions split and the database connection/schema-tree split.
  4-02 needs phase 2.
- **The cheap minors.** 1-13's last five imports, 6-03's collections-sync move and 4-01r's guard
  shape are each a few hours of work.

## Status of earlier findings

| ID | Sev (wave 0 → now) | Status | Evidence now |
|----|--------------------|--------|--------------|
| 1-01 (residual) `run_workflow_with_budget` | minor → — | **Fixed** | `workflow_engine.rs:1856` is a driver over `workflow_engine/run/{admission,state,schedule,finalize,identity}.rs`. `loc-baseline.json` `functions.run_workflow_with_budget: 93` (was 1,250). `run/mod.rs:1-7` documents the phase split. |
| 1-04 fan-out + summarizer per feature | major | Partly fixed, still open | **Fixed:** `otto-agent-run/src/fanout.rs` (357 lines: `run`, `summarize`, `Slot`, `SUMMARIZER_RETRY_IDX`) exists, and `skill_review.rs:45`, `:833-850` run on it. **Open:** `vault_docs_agent.rs:14` still says it "Mirrors the `product_run` fan-out + summarizer", and its own retry loop is at `:1323`, `:1375` and `:2389-2469`. Only the constant moved: `SUM_RETRY_IDX = otto_agent_run::fanout::SUMMARIZER_RETRY_IDX` (`:356`). `modules/review/summarize.rs::summarize_and_persist` is still 467 lines, and `otto-product/src/run.rs:6`, `:214` and `:278` still have their own summarizer. Three of the four engines are not ported. |
| 1-05 two MCP surfaces | minor | Partly fixed, still open | **Fixed:** four exact twins (`otto_get_session`, `otto_get_workflow_run`, `otto_list_findings`, `otto_list_prs`) go through `route_for` via `ROUTED_TWINS` / `twin_read` (`ottod/src/mcp_tools.rs:2538-2560`), and `read_route` is down to 422. **Open:** `route_for` 1,555, `run_tool` 1,189 and `base_tool_catalog` 687 are unchanged, and the near-twins (`otto_get_pr`, `:2652`, and the others) keep their own routes. |
| 1-07 (residual) REST engine in a route file | minor | Open | `routes/api_client.rs` is 3,261 lines. `execute` (`:1803`), `execute_service` (`:1821`) and `resolve_environment` (`:2012`) are still there, and they are still called from `workflow_api_request.rs:50` and `routes/api_automation_runs.rs:103`. FIXES P03: "not moved". |
| 1-08 (residual) `WorkflowsPage.svelte` | minor | Open | 3,710 lines, unchanged. `refetchRun` is at `:285` and `waitRunTerminal` at `:1003`. |
| 1-09 `GraphView.svelte` | major | Open | 4,879 lines (was 4,887; ui-guards `file-loc` 4,879). 6-04's typed client is in use (`git.refs` at `:720`, `:867`, `:956`, `:970`; `git.checkout` at `:1837`, `:1849`), but no RefSidebar or `actions.svelte.ts` has been extracted. |
| 1-10 `DatabaseStore` | major | Partly fixed, still open | 3,312 → 3,210 lines. The quick-filter chips moved to `lib/stores/database/filters.svelte.ts`. Connections and warm/keep-alive (`connEpoch`, `:170`, `:488`), the schema tree, object ops and query runs are still in `class DatabaseStore` (`:125`). |
| 1-13 `lib/` imports `modules/` | minor | Partly fixed, still open | 19 → 5. **Left:** `lib/stores/assistant.svelte.ts:19` (`modules/assistant/model`), `lib/stores/swarm.svelte.ts:8` (`modules/swarm/runStats`), `lib/uiCommands/vault.ts:13` (`modules/vault/vault.svelte`), and `lib/uiCommands/home.ts:11-12` (`modules/home/home.svelte`, `modules/home/kinds`). The router, canvas, product, apiClient, mockup-assist, k8s, browser, brokers, git and swarm-command imports are gone. |
| 2-06 rooms inside otto-server | minor | Open | `otto-server/src/rooms/` and `game_rooms/` are unchanged, and there is no `otto-rooms` crate. |
| 4-01 (residual) swarm paste + Enter | nit → **minor** | Partly fixed, still open; the fix added a mis-shaped API | **Added:** `otto-sessions/src/submit.rs:87-97` (`RunGuard(Arc<dyn Fn() -> bool>)`) and `:235` (`submit_prompt_while`). **Open:** nothing in production calls them. `rg submit_prompt_while\|RunGuard` outside tests finds only `submit.rs`. The swarm still assembles wait → paste → re-inject by hand (`runtime/run.rs:894-938`, `runtime/agent_run.rs:355-401`, host methods `host.rs:105-107`). Its run check is the async DB poll `while_run_active` (`engine.rs:49-63`, `run_is_active(repo, run_id).await`), which a synchronous `Fn() -> bool` cannot call. The hand-off in FIXES P01 cannot be done as written. See the detail below. |
| 4-02 three design-document assist engines | major | Partly fixed (all interim steps done), still open | **Fixed:** canvas and Design Hall claim through `otto_core::cancel_signal::InFlightSet` (`otto-design-assist/src/lib.rs:1029-1066`). `AgentTurn` lives at `otto_agent_run::session_turn::AgentTurn` (`session_turn.rs:131`). `otto-canvas/Cargo.toml` no longer depends on `otto-design-assist`. scene3d moved to `otto-design/src/format/scene3d.rs` (1,399), leaving 4-line re-exports. **Open (phase 2):** there are still three engines, `otto-canvas/src/assist.rs` (1,310), `otto-design-assist/src/lib.rs` (3,307) and `otto-server/src/mockup_assist.rs` (775), each with its own turn job and live poll (`assist.rs:740`, `lib.rs:1696-1707`, `mockup_assist.rs:457`). |
| 4-04 (residual) over-cap functions | nit → — | **Fixed** | `review_run.rs::execute` is no longer in `loc-baseline.json` `functions`. The file's largest functions are `resolve_targets` 177, `launch` 136, `plan_review` 135 and `score_target` 134. |
| 5-02 turn runner in otto-server | minor → nit | Partly fixed, residual | **Fixed:** `otto-agent-run/src/session_turn.rs` (1,340), `session_turn/lifecycle.rs` (582) and `session_turn/reply.rs` (137). `agent_session.rs` went from 1,359 to 265 lines of `&ServerCtx` wrappers, and `agent_session_{lifecycle,reply}.rs` were deleted. **Open:** the four `run_agent_turn` host forwards (`otto-canvas/src/assist_ctx.rs:56`, `otto-design-assist/src/ctx.rs:53`, `modules/ctx_impls.rs:545`, `:641`) and `require_owned_resume` (`agent_session.rs:25`) stay in the server. |
| 5-03 two MCP governance pipelines | minor | Partly fixed (doc), still open | **Fixed:** `docs/features/mcp-control-plane.md:45` now describes two entry points with different stage sets. **Open:** policy-as-code still runs only in `otto-mcp/src/service.rs`, and auto-approve only in `mcp_outward.rs:446`. The stage order (approval, dry-run, audit) is still written twice. FIXES P05: "the `otto_mcp::governance` extraction" not done. |
| 5-04 product `OverviewTab.svelte` | minor | Partly fixed, still open | 2,971 → 2,494 lines. `DraftEditor.svelte` (274) and `StoryTranscripts.svelte` (328) were extracted. The Jira issue section (`:81-83` `JiraIssueEditor`, `IssueFull`/`DevStatus` loads) is still inline. |
| 5-05 (residual) three.js stages | nit → — | **Fixed** | `design-hall/studio3d/embed.ts:31` imports `demandLoop, lossBudget, onContextLost, watchVisibility` from `lib/three/stage`. The context-loss remount is at `:330` and visibility at `:430`. The own `IntersectionObserver` is gone. |
| 6-01 five subsystems choose transcript roots | major → — | **Fixed** | `otto-transcript/src/roots.rs` (`TranscriptRoots::discover` / `for_session`, `RootsEnv`, `isolated_dir`). It is used by the usage tailer (`driver.rs:35`, `:117`, `:265`), ottod (`main.rs:524-539`), `history_index.rs:150-191`, `transcript/resolve.rs:18`, `:36`, `:111`, `otto-improve/src/evidence.rs:126` and `SessionManager::codex_root_for` (`manager.rs:1866-1876`). `codex_sessions_root` (`manager.rs:417`) delegates to `RootsEnv::default_codex_root`. One predicate, `otto_core::e2e::isolated` (`e2e.rs:9`), is the only `env::var("OTTO_E2E")` left in `crates/`, and a `check.sh` ratchet guards it. Tailer test at `driver.rs:2246-2254` (account and `$CODEX_HOME` rollouts). A few per-session path builders outside `roots` remain; that is new nit 6w1-01. |
| 6-02 two turn-open parsers | minor → — | **Fixed** | The folds live in `otto-transcript/src/turn.rs` (889 lines). `turn::tail_open` is at `:104`, and the one notification parser at `:525`. `SessionManager::agent_turn_open` (`manager.rs:1220-1222`) calls it. `claude_turn_open`, `codex_turn_open` and the manager's `TASK_NOTIFICATION` are gone. |
| 6-03 `apiClient.svelte.ts` god store | minor | Partly fixed, still open | 2,308 → 2,113 lines. Automations moved to `lib/stores/apiClient/automations.svelte.ts` (269) behind the façade (`:1777-1805`), and `graphqlVars` moved to `lib/api/`. The tab persistence and slots (`:218-650`), cookie jar (`:951`), collections with Postman/git sync (`:1001-1263`) and curl (`:1655`) are still in the class. |
| 6-04 no typed git client | minor → — | **Fixed** | `ui/src/lib/api/git.ts` provides `gitApi.{status, refs, readRefs, push, pushTarget, checkout, stage, pull, prs.*}`. The store owns the post-mutation contract (`git.svelte.ts:801`, refsRev bump at `:173`). Every listed site now goes through it: `CreatePr.svelte:255` (`git.push`), `pushFlow.ts:59-61`, `GitToolbar.svelte:171`, `GraphView.svelte:720`, `:867`, `:956`, `:970`, `:1837`, `:1849`, `LocalReviewPanel.svelte:169` and `uiCommands/git.ts:297`. |
| 6-05 list windowing ×8 | minor → nit | Partly fixed, residual | **Fixed:** `lib/listWindow.svelte.ts` (`ListWindow`, `listRange`, `findRows`) is the base. `lib/tableWindow.svelte.ts:26` builds on it. WipPanel (`:558-559`), FileTree (`:411`), the DiffViewer nav (`:369`) and BlamePanel (`:110`) register ⌘F providers. SchemaTree and GraphView use the shared range, and `docs/design/guidelines/components.md:229` states the rule. **Open:** `modules/kubernetes/monitor/tableWindow.ts` (61 lines) is still its own range math (FIXES P03 defers it). `modules/git/list-range.ts` and `modules/git/list-window.svelte.ts` are re-export shims with **no importers** anywhere in `ui/`. |

No earlier architecture finding was rejected.

## Areas covered in this iteration

These are the same areas as wave 0, re-read at `37738d908`:

- **Earlier findings.** Re-verified all 21 open wave-0 findings, as above.
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

**The wave-1 diff for this lens** (`git diff origin/main...HEAD`, 480 files) was read inside those
areas:

- the new crate modules: `otto-transcript/{roots,turn}.rs`, `otto-agent-run/{fanout,session_turn}`,
  `otto-core/e2e.rs` and `otto-design/format/scene3d.rs`;
- the dependency edges in `Cargo.toml`: otto-canvas dropped otto-design-assist, and otto-transcript
  and otto-agent-run gained otto-core and otto-transcript;
- the workflow-engine `run/` split and the dbviewer `ExportWriter` change to the `Driver` trait;
- the session-manager and `mcp.rs` changes and the ottod twin routing;
- the UI moves (`lib/api/git.ts`, `lib/listWindow.svelte.ts`, `lib/k8s/`,
  `lib/stores/apiClient/automations.svelte.ts`, `lib/stores/database/filters.svelte.ts`, the product
  extractions);
- every cross-module import the diff adds (script check: one, 6w1-02).

The fixes introduced no new dependency cycle and no wrong-way crate edge. One new API is unused and
mis-shaped (4-01r). Two shims are dead (6-05r).

## Areas NOT covered yet (next iteration)

Unchanged from wave 0: otto-usage `engine.rs` / `clickhouse.rs` internals; otto-dbviewer `service.rs`
/ `types.rs`; otto-git `local.rs` cohesion (the new `local/errors.rs` was not judged); the large
ratcheted functions (`run_analysis`, `controller_inner`, `bridge::handle`, `static_review`,
`run_skill_eval_core`, `route_result_inner`); the large UI files (`ReviewPanel`, `DatabasePage`,
`QueryEditor`, `QueryBuilder`, `UsagePage`, `RequestBuilder`, `DocsAgentsView`, `FloatingBar`,
`SessionView`); typed-client gaps in settings and workflows; hand-rolled drag-to-reorder. Not judged
either: the new `webhook_key_guard` living in `routes/channel_webhook.rs:134` and imported by
`swarm_webhook.rs`.

---

## Open findings

### architecture-6w1-01: nit: per-session Claude transcript paths are still built outside `otto_transcript::roots`

**Location:** `crates/otto-agent-run/src/session_turn.rs:1097-1112`; `crates/otto-sessions/src/manager.rs:471-492`; `crates/otto-sessions/src/lifecycle.rs:40-50`, `:124-135`; `crates/otto-orchestrator/src/claude_pty.rs:469-485`; `crates/otto-improve/src/pathsafe.rs:17`

**Evidence**

6-01 made `TranscriptRoots::for_session` the one answer to "where does this session's CLI write".
The per-session path builders that existed alongside it still compose the layout by hand:

- `session_turn::transcript_path` (`:1104-1108`) builds `<account home>/projects/<enc(cwd)>/<psid>.jsonl`.
  Without an account it calls `claude_pty::session_jsonl_path`, which reads `$HOME` directly
  (`claude_pty.rs:480`).
- `persist_transcript_path` (`manager.rs:479-492`) reads `HOME` itself. It joins
  `root.join("projects")` / `root.join("sessions")` for an account. Otherwise it calls
  `lifecycle::transcript_path(home, …)`, which hard-codes `home/.claude/projects` (`lifecycle.rs:130`).
- There are three cwd encoders: `lifecycle::claude_project_dir_name` (`:40`), `claude_pty::project_dir`
  (`:475`, whose doc says it "mirrors" the other) and `otto-improve/src/pathsafe.rs:17`.

None of these honour `OTTO_TRANSCRIPT_ROOTS` or the isolated fixture. The resolve/History/Usage
paths do. So under those overrides the turn watcher and the persisted `transcript_path` look in a
different place than the conversation view.

**Cost**

This is small today, because the overrides are test and e2e settings. But the next change to the
account layout or the cwd encoding has to be made in five places again, which is the drift 6-01
removed for the root lists.

**Fix**

Add `TranscriptRoots::claude_session_file(cwd, psid) -> Option<PathBuf>` (first existing, else the
first root's expected path) and one `encode_cwd` in `otto-transcript`. Make `session_turn`,
`persist_transcript_path` and `lifecycle::claude_transcript_path` call it, and have
`claude_pty::project_dir` delegate to it.

### architecture-6w1-02: nit: the database module reaches into `modules/api` for a generic focus helper

**Location:** `ui/src/modules/database/AggregateBuilder.svelte:15`; `ui/src/modules/api/rowFocus.ts:1-15`

**Evidence**

Wave 1 added `import { focusAfterRowRemoval } from '../api/rowFocus'` to AggregateBuilder. This is
the only cross-module import the diff adds. `rowFocus.ts` is a generic a11y helper for index-keyed
row editors, with no API-client knowledge. It lives in the API client module, and four API files
import it locally.

**Cost**

The database module now depends on the API client module's internals. The next list editor (brokers,
workflows) will reach into one of those two modules too, and moving or renaming the file breaks
unrelated modules.

**Fix**

Move `rowFocus.ts` to `ui/src/lib/rowFocus.ts`, next to `focusOnMount` and `inlineEdit`, and update
its five importers.

### architecture-4-01 (residual, now minor): swarm paste path unconverted, and the new `RunGuard` cannot serve it

**Location:** `crates/otto-sessions/src/submit.rs:80-97`, `:235-280`; `crates/otto-swarm/src/runtime/run.rs:894-938`; `crates/otto-swarm/src/runtime/agent_run.rs:355-401`; `crates/otto-swarm/src/runtime/engine.rs:49-74`; `crates/otto-swarm/src/runtime/host.rs:105-107`

**Evidence**

`RunGuard(Arc<dyn Fn() -> bool + Send + Sync>)` and `SessionManager::submit_prompt_while` were added
for "a swarm run, a stoppable engine step" (`submit.rs:80-82`). Neither has a production caller. The
swarm still drives `wait_for_tui` → `bracketed_paste` → re-inject itself, through three host methods.
Its run-active check is `run_is_active(repo, run_id).await`, an async SQLite read polled every 250 ms
(`engine.rs:49-63`). A synchronous `Fn() -> bool` cannot perform it without a cached flag that some
other task keeps fresh.

**Cost**

The codebase now has two run-guarded submit paths: the swarm's, which is live, and the session
manager's, which is unused. The API meant to replace the swarm's path does not fit it, so it either
stays dead or gets bent at the first real adoption.

**Fix**

Make the guard async-capable, for example `RunGuard(Arc<dyn Fn() -> BoxFuture<'static, bool> + Send
+ Sync>)`, or a `watch::Receiver<bool>` that the swarm engine flips on stop. Convert `run.rs` and
`agent_run.rs` to `submit_prompt_while(.., SubmitOpts::VERIFIED, guard)`. Then delete the swarm
host's `bracketed_paste` and `wait_for_tui` and keep only `dispatched`.

### architecture-6-05 (residual, now nit): k8s table window and two dead git shims

**Location:** `ui/src/modules/kubernetes/monitor/tableWindow.ts:1-61`; `ui/src/modules/git/list-range.ts`; `ui/src/modules/git/list-window.svelte.ts`

**Evidence**

The k8s monitor keeps its own `tableWindow` range math (FIXES P03: it is off by one row from
`listRange` and needs an `extra` band first). The two git files are one-line re-exports of
`lib/listWindow.svelte`, and `rg "list-range|list-window" ui` finds no importer.

**Fix**

Delete the two shims. Add an "extra height after index i" option to `listRange` and re-express the
k8s window over it.

### architecture-1-04: major (partly fixed): three engines still hand-roll fan-out and summarizer

**Location:** `crates/otto-server/src/vault_docs_agent.rs:14`, `:356`, `:1323-1375`, `:2389-2469`; `crates/otto-server/src/modules/review/summarize.rs` (`summarize_and_persist` 467); `crates/otto-product/src/run.rs:6`, `:214-320`

**Remaining fix:** port the vault-docs writer pass and the code-review summarizer onto
`otto_agent_run::fanout::run` / `summarize`, then product analysis. Delete each engine's own retry
index handling.

### architecture-1-09: major: `GraphView.svelte` 4,879 lines

**Location:** `ui/src/modules/git/GraphView.svelte`

**Remaining fix:** the typed git client (6-04) is now in place. Extract `RefSidebar.svelte` and
`graph-actions.svelte.ts` (checkout, branch, tag, stash and push handlers over `git.*`), then lower
the `file-loc` baseline.

### architecture-1-10: major (partly fixed): `DatabaseStore` 3,210 lines

**Location:** `ui/src/lib/stores/database.svelte.ts:125`, `:170`, `:488`

**Remaining fix:** split connections and warm/keep-alive and the schema tree into
`lib/stores/database/{connections,schemaTree}.svelte.ts`, sharing `connEpoch` as a token object.
Then do object ops and query runs.

### architecture-4-02: major (interim done): three design-document assist engines

**Location:** `crates/otto-canvas/src/assist.rs` (1,310, live poll `:740`), `crates/otto-design-assist/src/lib.rs` (3,307, `spawn_live_poll` `:1696`), `crates/otto-server/src/mockup_assist.rs` (775, `:457`)

**Remaining fix:** phase 2. Build one `AssistTarget` trait (load, prompt, parse, apply, live-file path)
over a shared turn job and live poll in `otto-agent-run`, and make the three engines adapters.

### architecture-1-05: minor (partly fixed): two MCP tool surfaces

**Location:** `crates/otto-mcp/src/outward/exec.rs::route_for` (1,555), `crates/ottod/src/mcp_tools.rs::run_tool` (1,189), `::base_tool_catalog` (687), `:2652` (`otto_get_pr` and the other near-twins)

**Remaining fix:** extend `ROUTED_TWINS` as near-twins converge, and generate the session catalog
from the control-plane specs.

### architecture-1-07 (residual): minor: REST execute engine in a route file

**Location:** `crates/otto-server/src/routes/api_client.rs:1803`, `:1821`, `:2012`; called by `workflow_api_request.rs:50`, `routes/api_automation_runs.rs:103`

**Remaining fix:** move `execute_service` and `resolve_environment` into `otto-apiclient` behind an
`ApiClientCtx` trait. The route keeps the axum handler.

### architecture-1-08 (residual): minor: `WorkflowsPage.svelte` 3,710 lines

**Location:** `ui/src/modules/workflows/WorkflowsPage.svelte:285`, `:1003`

**Remaining fix:** move the live run sync, final output and approvals into `RunMonitor.svelte`.

### architecture-1-13: minor (partly fixed): five `lib → modules` imports

**Location:** `ui/src/lib/stores/assistant.svelte.ts:19`, `ui/src/lib/stores/swarm.svelte.ts:8`, `ui/src/lib/uiCommands/vault.ts:13`, `ui/src/lib/uiCommands/home.ts:11-12`

**Remaining fix:** move `assistant/model` and `swarm/runStats` (pure) into `lib/`. Have the vault and
home command handlers registered by their modules at mount, or move those stores into
`lib/stores/`. Then add a ui-guards rule with max 0.

### architecture-2-06: minor: rooms inside otto-server

**Location:** `crates/otto-server/src/rooms/`, `crates/otto-server/src/game_rooms/`

**Remaining fix:** an `otto-rooms` crate behind a `RoomsCtx`. The capability step (5-01) is done.

### architecture-5-03: minor (doc fixed): two MCP governance pipelines

**Location:** `crates/otto-mcp/src/service.rs` (policy-as-code), `crates/otto-server/src/mcp_outward.rs:446` (auto-approve)

**Remaining fix:** extract the shared stage runner (`otto_mcp::governance`: approval, dry-run, audit)
that both entry points call with their own stage sets. Make the decision on policy for `otto.*`
tools explicit.

### architecture-5-04: minor (partly fixed): `OverviewTab.svelte` 2,494 lines

**Location:** `ui/src/modules/product/OverviewTab.svelte:81-83` and the Jira/dev-status section

**Remaining fix:** extract `JiraIssueSection.svelte` (issue load, dev status, IssueEditor host).

### architecture-6-03: minor (partly fixed): `apiClient.svelte.ts` 2,113 lines

**Location:** `ui/src/lib/stores/apiClient.svelte.ts:218-650`, `:951`, `:1001-1263`, `:1655`

**Remaining fix:** follow the automations pattern. Next are
`lib/stores/apiClient/collectionsSync.svelte.ts` (Postman and git pull/push), then tabs and cookies.
Lower `file-loc` after each.

### architecture-5-02 (residual): nit: four host forwarders and the resume-ownership check in the server

**Location:** `crates/otto-canvas/src/assist_ctx.rs:56`, `crates/otto-design-assist/src/ctx.rs:53`, `crates/otto-server/src/modules/ctx_impls.rs:545`, `:641`, `crates/otto-server/src/agent_session.rs:25`

**Remaining fix:** give `session_turn` a `ResumeAuthority` hook (or pass the `SessionsRepo`) so
`require_owned_resume` moves with it. The ctx traits can then expose `&Arc<SessionManager>` instead
of forwarding `run_agent_turn`.
