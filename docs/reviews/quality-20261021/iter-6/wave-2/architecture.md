# Architecture review: quality loop 2026-10-21, iteration 6 (wave 2)

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

This is a re-score after fix wave 2. It was read-only: `git diff 37738d908..HEAD` (the wave-2 diff),
`git diff origin/main...HEAD` (the whole branch), `git show`, `rg`/`grep`, `sed`, `wc` and a Python
read of `scripts/loc-baseline.json` and `ui/scripts/ui-guards-baseline.json`. Nothing was built or
tested. Every status was confirmed against `fix/quality-iter6-20261023` at `4f6d6ab30`.

Excluded: `target/`, `node_modules/`, `docs/reviews/`. The scope is the same as the wave-1 report:
the same areas at the same depth, plus the wave-2 diff read for this lens.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Earlier findings still open, fully or as a residual | 0 | 1 (1-09) | 5 (1-04r, 1-05, 1-07r, 2-06, 6-03) | 1 (4-01r) |
| New in wave 2 | 0 | 0 | 0 | 2 (6w2-01, 6w2-02) |
| **Total** | 0 | 1 | 5 | 3 |

**Score: 10.0 − 1 × 0.3 − 5 × 0.1 − 3 × 0.03 = 10.0 − 0.3 − 0.5 − 0.09 = 9.11 / 10**

The score went from 7.78 to 9.11. Wave 2 fully fixed ten of the seventeen open findings: 4-02 and
1-10 (two majors), 1-08r, 1-13, 5-03 and 5-04 (four minors), and 5-02r, 6-05r, 6w1-01 and 6w1-02
(four nits). Two more moved down:

- **1-04 went from major to minor.** The vault-docs writer pass and the code-review summarizer now
  run on `otto_agent_run::fanout` (plus the new `fanout::retry`). Only product analysis still has
  its own fan-out and summarizer.
- **4-01r went from minor to nit.** `RunGuard` is async and the swarm's turns submit through
  `submit_prompt_while`. What is left is the old paste path, which is kept public only for one test.

Two new nits come from the fixes. 6w2-01 covers the two cwd-encoding and Claude-layout copies that
6w1-01's fix left behind. 6w2-02 is a type/component name collision created by the RunMonitor split.

The only major still open is 1-09: GraphView is 4,082 lines and the RefSidebar extraction is not
done. Extracting it, then splitting the apiClient store (6-03) and moving the REST engine (1-07r),
would lift the score to about 9.5.

## Status of earlier findings

| ID | Sev (wave 1 → now) | Status | Evidence now |
|----|--------------------|--------|--------------|
| 1-04 fan-out + summarizer per feature | major → **minor** | Partly fixed, still open | **Fixed:** vault docs runs its writer pass on `fanout::run_slots` (`vault_docs_agent/run.rs:153`), its summarizer on `fanout::summarize_with_retries` (`run.rs:295`), and its reviewer and revision loops on `fanout::attempts` (`review_loop.rs:330`, `:500`). `SUM_RETRY_IDX`, `close_retry` and `wait_for_retry_flag` are gone. The file was split into `vault_docs_agent/{authors,run,review_loop,retry,refine,prompts}.rs`, and the root is 1,379 lines (was 4,093). Code review: `summarize_and_persist` (`modules/review/summarize.rs:29`) is now an adapter over `fanout::summarize` (`:115`), and the retry paths map rows through `retry::row_slot` (`modules/review/retry.rs:4`). New shared module: `otto-agent-run/src/fanout/retry.rs` (336). **Open:** `otto-product/src/run.rs` still has its own lens fan-out and summarizer session (`run_analysis` `:495`, 575 lines; summarizer at `:756-850`; `build_summarizer_prompt` `:396`). The file is 3,598 lines (3,574 on main). |
| 1-05 two MCP surfaces | minor | Open | Unchanged in wave 2. `route_for` (`otto-mcp/src/outward/exec.rs:522`) is 1,555 lines, `run_tool` (`ottod/src/mcp_tools.rs:3092`) 1,189 and `base_tool_catalog` (`:968`) 687, per `loc-baseline.json`. `ROUTED_TWINS` (`:2540`) still holds four entries, and near-twins such as `otto_get_pr` keep their own route (`:2652`). The merged `otto.query_db` was added to `route_for` (`exec.rs:470-516`, `:560`) without making the duplication worse. FIXES P4: "not started". |
| 1-07r REST engine in a route file | minor | Open | `routes/api_client.rs` is still 3,261 lines. `execute` is at `:1803`, `execute_service` at `:1821` and `resolve_environment` at `:2012`. They are still called from `workflow_api_request.rs:50` and `routes/api_automation_runs.rs:103`. FIXES P1: "not started". The verifier raised the otto-server LOC total by +324 and named this move as the payback. |
| 1-08r `WorkflowsPage.svelte` | minor → — | **Fixed** | 3,710 → 2,688 lines. Live sync, approvals and final output now live in `runSync.svelte.ts` (395, class at `:77`), `RunMonitor.svelte` (247), `RunApprovals.svelte` (146), `RunFinalOutput.svelte` (56), `RunContextSidebar.svelte` (161) and `RunInputPanel.svelte` + `runInputForm.svelte.ts`. The page only calls `mon.refetchRun` (`:794`) and `mon.waitRunTerminal` (`:807`). See the new nit 6w2-02 about naming. |
| 1-09 `GraphView.svelte` | major | Partly fixed, still open | 4,879 → 4,082 lines. Actions moved to `git/graph/graphActions.svelte.ts` (504) and menus to `git/graph/graphMenus.ts` (474), behind a `GraphActionsHost` (`graphActions.svelte.ts:30`). **Open:** the refs sidebar `<aside>` (`GraphView.svelte:1985-2376`, plus its CSS and sidebar-only script), the ref popover and the commit-detail pane are still inline. FIXES P5: "not yet ≤ 3,000 lines". |
| 1-10 `DatabaseStore` | major → — | **Fixed** | `lib/stores/database.svelte.ts` is a 1,004-line façade over `database/{connections (778), schemaTree (340), objectOps (261), queryRuns (773), workbenchPersist (408)}.svelte.ts` (imports `:93-97`). Each part has a narrow host contract (`import type { DatabaseStore }`, `connections.svelte.ts:24`), and they share one `ConnEpochs` token (`connections.svelte.ts:35`). |
| 1-13 `lib/` imports `modules/` | minor → — | **Fixed** | `rg` for imports of `modules/` under `ui/src/lib` finds only comments. The moves: `lib/assistant/model.ts`, `lib/home/kinds.ts`, `lib/swarm/runStats.ts`, plus `lib/uiCommands/{homePort,vaultPort}.ts` over one `createPagePort`. The old module files are deleted and no shims are left. The ui-guards `lib-imports-modules` rule is now hard (max 0). |
| 2-06 rooms inside otto-server | minor | Open | `otto-server/src/rooms/` (15 entries) and `game_rooms/` (7) are unchanged in wave 2, and there is no `otto-rooms` crate. FIXES P3: "not started". |
| 4-01r swarm paste + Enter | minor → **nit** | Partly fixed, residual | **Fixed:** `RunGuard` now wraps an async `Fn() -> Pin<Box<dyn Future<Output = bool>>>` (`otto-sessions/src/submit.rs:89-110`). `engine::run_guard` (`otto-swarm/src/runtime/engine.rs:65-90`) covers the DB status and the cancel flag. Both turn paths submit through `submit_turn_prompt` → `submit_prompt_while(.., VERIFIED, guard)` (`run.rs:943`, `:970`, `agent_run.rs:361`, `:391`). `bracketed_paste` and `wait_for_tui` are gone from `SwarmHost` (`host.rs:104-107`). **Residual:** the old paste → Enter path is still public, used only by a test. See the residual below. |
| 4-02 three design-document assist engines | major → — | **Fixed** | `otto-agent-run/src/assist/mod.rs` (336) provides `AssistTarget`, `TurnRunner`, `AssistSlots`/`AssistClaim` and `run_assist_job` (`:216`), with the one live poll at `:307`. Canvas `SceneTurn` (`assist.rs:209`), Design Hall `DesignTurn` (`otto-design-assist/src/job.rs:81`) and `MockupTurn` (`mockup_assist.rs:209`) are adapters. `spawn_live_poll` and the per-engine pollers are gone. The canvas `assist_preview` throwaway turn (`assist.rs:459`) runs `session_turn::run_agent_turn` directly, which is right: it has no document to own. |
| 5-02r host forwarders + resume check in server | nit → — | **Fixed** | `CanvasAssistCtx::sessions()` (`assist_ctx.rs:60`) and `DesignAssistCtx::sessions()` (`ctx.rs:55`) replace the `run_agent_turn` / `kill_session` forwarders. `ctx_impls.rs:545` and `:618` return the manager. `ResumeAuthority` and `require_owned_resume` now live in `otto-agent-run/src/session_turn/resume.rs` (173). The server's `agent_session::require_owned_resume` (`:27-33`) is a one-line delegate. |
| 5-03 two MCP governance pipelines | minor → — | **Fixed** | `otto-mcp/src/governance.rs` (438) is the one stage runner, with a fixed order and two named stage sets, `REGISTERED_SERVERS` and `OTTO_TOOLS` (`:37-50`). `McpService::invoke` (`service.rs:843`, stage bodies in `service_stages.rs:34`) and `governed_invoke` (`mcp_outward.rs:456`, `OttoStages` `:609-680`) both call `governance::run`. The policy decision for `otto.*` is now explicit in `OTTO_TOOLS`'s doc. |
| 5-04 product `OverviewTab.svelte` | minor → — | **Fixed** | 2,494 → 1,772 lines. The Jira section is now `product/JiraIssueSection.svelte` (756) over `product/jiraIssue.svelte.ts` (112). |
| 6-03 `apiClient.svelte.ts` god store | minor | Open | 2,113 lines, unchanged in wave 2. The cookie jar is at `:955-968`, Postman/git sync at `:1117-1263` and curl at `:1658`/`:1692`, all still in the class. FIXES P7: "not started". |
| 6-05r k8s table window + dead git shims | nit → — | **Fixed** | `listRange` now takes an `extra` band (`lib/listWindow.svelte.ts:45-69`), and `kubernetes/monitor/tableWindow.ts:41-50` maps onto it. `git/list-range.ts` and `git/list-window.svelte.ts` are deleted. |
| 6w1-01 per-session Claude paths outside `roots` | nit → — | **Fixed** (every listed location) | `TranscriptRoots::claude_session_file` and `roots::encode_cwd` (`otto-transcript/src/roots.rs:46`) now back all of these: `session_turn::transcript_path`, `persist_transcript_path` (moved to `otto-sessions/src/manager/transcript_roots.rs:21`, using `transcript_roots_for`/`_in`), `lifecycle::claude_project_dir_name` (now a `pub use` of `encode_cwd`, `lifecycle.rs:39`), `claude_transcript_path` (`:43`), `lifecycle::transcript_path` (`:118-134`, over `RootsEnv`), `claude_pty::{project_dir, session_jsonl_path}` (`claude_pty.rs:480-490`) and `otto-improve/src/pathsafe.rs:17-19`. Two copies outside the listed locations are left; they are the new nit 6w2-01. |
| 6w1-02 database → `modules/api/rowFocus` | nit → — | **Fixed** | The helper is now `ui/src/lib/rowFocus.ts`. `modules/api/rowFocus.ts` is gone, and its five importers (four API files and `database/AggregateBuilder.svelte`) import from `lib/`. |

No earlier architecture finding was rejected.

## Areas covered in this iteration

These are the same areas as wave 1 (and wave 0), re-read at `4f6d6ab30`:

- **Earlier findings.** Re-verified all 17 findings open at wave 1, as above.
- **Recent churn.** Ranked every file by commits in the last 3 weeks and checked the top 50: `types.ts`
  117, `otto-sessions/manager.rs` 51, `otto-server/modules.rs` 48, `policy.rs` 42,
  `workflow_engine.rs` 41, `otto-git/local.rs` 40, `ottod/mcp_tools.rs` 33, `workspace.svelte.ts`
  33, and so on. Also checked growth over 3 and 6 weeks for the large UI files.
- **Transcript discovery and parsing across crates.** `otto-usage/src/tailer/driver.rs`,
  `otto-server/src/{transcript/resolve.rs, history_index.rs}`, `otto-improve/src/evidence.rs`,
  `otto-sessions/src/{manager.rs, lifecycle.rs, accounts.rs}`, `otto-agent-run/src/turn_oracle.rs`
  and `ottod/src/main.rs::start_usage_tailer`. This produced 6-01 and 6-02, and this wave's 6w2-01.
- **otto-sessions `manager.rs`** (6,502 lines, ratcheted at 6,499 after the verifier moved the
  transcript-root helpers to `manager/transcript_roots.rs`). Outlined again. Provider-specific
  transcript code is mixed into the session manager; that is the 6-02 half. The rest (spawn, attach,
  authority, sweep and suspend) is the manager's own job. No further finding.
- **otto-dbviewer drivers.** `mysql.rs`, `postgres.rs`, `clickhouse.rs`, `mongodb.rs` and `redis.rs`
  share `sql_class::ReadClass`, `sql_common::{assemble_bulk_graph, acquire_error,
  float_to_json}`, and the `complete::sql::assemble` completion cache through thin trait methods.
  What each driver keeps is wire-specific. The wave-2 `service.rs` change (the `otto.query_db`
  merge: `guard_setting_hint`, agent-aware refusal text) is message wording inside the existing
  gate. No finding.
- **UI stores and large files.** `lib/stores/apiClient.svelte.ts` (6-03). The `/repos/{id}/…` call
  sites across the git module (6-04; the raw `api.post` calls moved into `graphActions.svelte.ts`
  were already raw in GraphView on main, outside 6-04's listed sites). The windowing primitives in
  `lib/`, `git/`, `kubernetes/`, `database/` and `panels/` (6-05). `git/DiffViewer.svelte` was
  outlined: no separate finding.
- **`ui/src/lib/api/types.ts`** (117 commits). It is already a façade over `lib/api/types/*.ts` and
  per-feature type files, and it churns because it mirrors the contract. No finding.
- **`otto-server/src/modules.rs`** (48 commits). It is a router-assembly seam whose children are
  already split. In wave 2, `ctx_impls.rs` lost its turn forwarders (5-02r). No new finding.
- **`policy.rs`.** This is the deliberately central route → feature table, fail-closed and
  coverage-tested. No finding.
- **`ottod/src/main.rs`.** Process setup, listeners and teardown. Only the usage-tailer wiring was
  relevant (6-01). No change in wave 2.
- **New domain files in the window.** `otto-automation/scheduled_tasks_engine.rs`,
  `otto-insights/lib.rs`, `otto-apiclient/{rest,grpc}.rs`, `otto-pty/holder.rs` and
  `otto-issues/linear.rs` are in their domain crates, behind their ctx traits. No finding.

**The wave-2 diff for this lens** (`git diff 37738d908..HEAD`, 69 crate files, plus the UI moves)
was read inside those areas:

- The new crate modules: `otto-agent-run/src/{assist/mod.rs, fanout/retry.rs,
  session_turn/resume.rs}`, `otto-mcp/src/{governance.rs, service_stages.rs}`,
  `otto-design-assist/src/job.rs` and `otto-sessions/src/manager/transcript_roots.rs`.
- The dependency edges. `otto-agent-run` promotes `otto-state` from a dev-dependency to a
  dependency, and `otto-orchestrator` gains `otto-transcript`. Both are leaf-ward. There is no
  cycle and no edge into otto-server.
- The vault-docs split into `vault_docs_agent/*` and the `review/summarize.rs` rewrite (467-line
  function → 148-line adapter plus named steps).
- The swarm submit conversion (`engine.rs`, `run.rs`, `agent_run.rs`, `host.rs`).
- The `otto.query_db` merge in `outward/exec.rs` (`db_query_call` shared by the read and write
  tools).
- The UI moves: the `database/*` store parts, `git/graph/*`, the `workflows/Run*` components and
  `runSync.svelte.ts`, `product/JiraIssueSection` + `jiraIssue.svelte.ts`, `lib/{rowFocus,
  assistant, home, swarm}`, `lib/uiCommands/{homePort,vaultPort,pagePort}` and `listRange`'s
  `extra` band.
- Every cross-module import the diff adds (script check). Three were found
  (`workflows/RunMonitor.svelte` and `RunContextSidebar.svelte` → `panels/FileTree.svelte`, and
  `workflows/runSync.svelte.ts` → `product/types`). All three are moves of imports `WorkflowsPage`
  already had on main, so none is new debt.

The fixes introduced no dependency cycle and no wrong-way crate edge. Two small problems are new:
a symbol/component name collision (6w2-02) and the two encoder/layout copies 6w1-01's fix left
(6w2-01). The test-only legacy paste path (4-01r) remains.

## Areas NOT covered yet (next iteration)

Unchanged from wave 1: otto-usage `engine.rs` / `clickhouse.rs` internals; otto-dbviewer `service.rs`
/ `types.rs` as a whole; otto-git `local.rs` cohesion (the new `local/errors.rs` was not judged);
the large ratcheted functions (`run_analysis`, `controller_inner`, `bridge::handle`,
`static_review`, `run_skill_eval_core`, `route_result_inner`); the large UI files (`ReviewPanel`,
`DatabasePage`, `QueryEditor`, `QueryBuilder`, `UsagePage`, `RequestBuilder`, `DocsAgentsView`,
`FloatingBar`, `SessionView`); typed-client gaps in settings and workflows; hand-rolled
drag-to-reorder; `webhook_key_guard` in `routes/channel_webhook.rs:134`. Also not judged:
`otto-design-assist/src/lib.rs` (2,743 lines after the job moved out) beyond its assist job, and
`mcp_outward.rs::governed_invoke` (395 lines, just under the 400 cap).

---

## Open findings

### architecture-1-09: major (partly fixed): `GraphView.svelte` 4,082 lines

**Location:** `ui/src/modules/git/GraphView.svelte:1985-2376` (refs sidebar `<aside>`), plus the ref
popover and the commit-detail pane in the same file

**Evidence**

Wave 2 extracted `graph/graphActions.svelte.ts` (504) and `graph/graphMenus.ts` (474), taking the
file from 4,879 to 4,082 lines. The refs sidebar (about 390 lines of markup at `:1985-2376`, plus
its CSS and sidebar-only state), the ref popover and the commit-detail pane are still in the one
component. It is still the largest Svelte file in the git module.

**Cost**

Every change to the sidebar, popover or detail pane edits and re-reviews a 4,000-line component.
State shared between those parts and the graph canvas has no boundary, so a change in one can break
another unnoticed.

**Fix**

Extract `git/graph/RefSidebar.svelte`, which takes the refs tree and `GraphActions` as props, then
the ref popover and `CommitDetailPane.svelte`. Lower the `file-loc` baseline after each extraction.

### architecture-1-04: minor (partly fixed, was major): product analysis still hand-rolls fan-out and summarizer

**Location:** `crates/otto-product/src/run.rs:495` (`run_analysis`, 575 lines), `:396`
(`build_summarizer_prompt`), `:756-850` (its own summarizer session)

**Evidence**

Vault docs, code review and skill review now share `otto_agent_run::fanout::{run, run_slots,
summarize, summarize_with_retries, attempts}`. Product analysis still runs its lens futures and its
summarizer session by hand. That includes the "summarizer row stuck in running" guard at
`:804-806`, the kind of edge the shared runner now owns.

**Cost**

The next change to fan-out semantics (retry, cancel, fallback) has to be made twice, and product
will drift from the other three engines.

**Fix**

Follow FIXES P1's next step: pass the lens futures as reviewers, `build_summarizer_prompt` plus the
summarizer session as `summarizer`, and the existing merge as `fallback` to `fanout::run` /
`fanout::summarize`. Then drop `run_analysis`'s `loc-baseline` entry.

### architecture-1-05: minor: two MCP tool surfaces

**Location:** `crates/otto-mcp/src/outward/exec.rs:522` (`route_for`, 1,555 lines),
`crates/ottod/src/mcp_tools.rs:3092` (`run_tool`, 1,189), `:968` (`base_tool_catalog`, 687),
`:2540` (`ROUTED_TWINS`, 4 entries), `:2652` (`otto_get_pr` and the other near-twins)

**Evidence**

Nothing changed in wave 2 (FIXES P4: "not started"). Each new outward tool, such as this wave's
`otto.query_db`, has to be checked against both catalogs and the two byte budgets.

**Fix**

Extend `ROUTED_TWINS` as the near-twins converge. Generate the session catalog from
`otto_tool_specs`, and add a parity test that fails when a control-plane tool is missing from the
session list.

### architecture-1-07 (residual): minor: REST execute engine in a route file

**Location:** `crates/otto-server/src/routes/api_client.rs:1803` (`execute`), `:1821`
(`execute_service`), `:2012` (`resolve_environment`); called from `workflow_api_request.rs:50` and
`routes/api_automation_runs.rs:103`

**Evidence**

The file is 3,261 lines and unchanged. Two non-route modules reach into a route module for the
engine. The verifier raised the otto-server LOC total by +324 and named this move as the payback,
so the debt is now also blocking the LOC ratchet.

**Fix**

Move `execute_service`, `resolve_environment`, `run_step` and `prepare_stream` into
`otto-apiclient` behind an `ApiClientCtx` trait. Keep only the axum handler in the route, then lower
the otto-server total.

### architecture-2-06: minor: rooms inside otto-server

**Location:** `crates/otto-server/src/rooms/` (15 entries), `crates/otto-server/src/game_rooms/`
(7)

**Evidence**

Unchanged; there is no `otto-rooms` crate (FIXES P3: "not started").

**Fix**

Create an `otto-rooms` crate behind a `RoomsCtx`. The capability step (5-01) is already done.

### architecture-6-03: minor (partly fixed): `apiClient.svelte.ts` 2,113 lines

**Location:** `ui/src/lib/stores/apiClient.svelte.ts:955-968` (cookies), `:1117-1263` (Postman/git
sync), `:1658`, `:1692` (curl), `:218-650` (tabs and slots)

**Evidence**

Unchanged in wave 2 (FIXES P7: "not started"). Only automations have been moved out, to
`apiClient/automations.svelte.ts`.

**Fix**

Follow the automations pattern: first `apiClient/collectionsSync.svelte.ts`, then tabs, cookies and
curl. Lower `file-loc` after each.

### architecture-4-01 (residual, now nit): the old swarm paste path is kept public for one test

**Location:** `crates/otto-swarm/src/runtime/engine.rs:118-129` (`guarded_dispatch`), `:131-145`
(`send_run_input`), `:147-183` (`#[doc(hidden)] pub async fn submit_run_prompt`);
`crates/otto-server/tests/swarm_router.rs:27`, `:461`

**Evidence**

Production turns now submit only through `submit_turn_prompt` → `submit_prompt_while`. The old
paste → wait → Enter → re-Enter sequence (`submit_run_prompt`) is still a public API, and its doc
says "It stays only for otto-server's `swarm_router` interleave test". `send_run_input` has no
other caller. So the crate still carries a second, hand-rolled paste/Enter path that no production
code uses.

**Cost**

The interleave test exercises this dead path, not the live one. A change to the submit-lock or
re-inject rules in `submit_prompt_while` would leave the test green.

**Fix**

Port the `swarm_router` interleave test to drive `submit_turn_prompt` (or
`SessionManager::submit_prompt_while` with `SubmitOpts` that skip the TUI wait for the `cat` PTY).
Then delete `submit_run_prompt` and `send_run_input`, and make `guarded_dispatch` private to its
remaining caller and test.

### architecture-6w2-01: nit: two Claude layout and cwd-encoder copies left outside `otto_transcript::roots`

**Location:** `crates/otto-sessions/src/nested.rs:282-286` (production path, called from
`manager.rs:3772`); `crates/otto-context/src/materialize.rs:395-411` (`enc_project`,
`read_memory`), also used at `:145`

**Evidence**

6w1-01's fix made `roots::encode_cwd` / `default_claude_root` / `claude_session_file` the one Claude
layout, and every listed location now uses it. Two copies were not in that list and are left:

- `nested::claude_transcript_in_window` builds `home.join(".claude").join("projects")` by hand. It
  uses the shared encoder through `lifecycle::claude_project_dir_name`, but not the session's
  `TranscriptRoots`. So a nested Claude agent under a provider account, or under
  `OTTO_TRANSCRIPT_ROOTS` or the isolated fixture, is looked for under `$HOME`.
- `otto-context`'s `enc_project` is a fourth copy of the encoder. Its doc says it "Mirrors
  `otto_orchestrator::claude_pty::project_dir`". `read_memory` reads `HOME` and hard-codes
  `.claude/projects`.

FIXES P3 lists both as hand-offs that were left open.

**Cost**

The next change to the encoding or the account layout has to find these two copies again, which is
the drift 6-01 and 6w1-01 removed. The nested-agent lookup is already wrong for account sessions.

**Fix**

Give `claude_transcript_in_window` a `&TranscriptRoots` (from `transcript_roots_for(session)` at
`manager.rs:3772`) and scan each Claude root's `encode_cwd(cwd)` directory. In `otto-context`, call
`otto_transcript::roots::{encode_cwd, default_claude_root, home_dir}` and delete `enc_project`.
otto-context then gains an `otto-transcript` dependency, which is leaf-ward.

### architecture-6w2-02: nit: the run-state class and the component share the name `RunMonitor`

**Location:** `ui/src/modules/workflows/runSync.svelte.ts:77` (`export class RunMonitor`);
`ui/src/modules/workflows/RunMonitor.svelte:16`, `:19`; `WorkflowsPage.svelte:25`, `:28`;
`RunApprovals.svelte:13`, `RunFinalOutput.svelte:8`, `RunContextSidebar.svelte:13`

**Evidence**

The verifier renamed the file `runMonitor.svelte.ts` → `runSync.svelte.ts` because it collided by
case with `RunMonitor.svelte`, but kept the class name. So `WorkflowsPage` has to alias it
(`import { RunMonitor as RunMonitorState } from './runSync.svelte'`) next to
`import RunMonitor from './RunMonitor.svelte'`. The `RunMonitor.svelte` component takes a prop typed
`RunMonitor`, which is its own name. Four components and the unit test (`w2p7-runMonitor.test.ts:59`)
use the class name.

**Cost**

One name now means two things. Every new importer has to choose an alias, and grep or
go-to-symbol for `RunMonitor` returns both the component and the state.

**Fix**

Rename the class `RunSync` (or `RunState`) to match its file. Update the six importers and the unit
test, and drop the alias in `WorkflowsPage.svelte:28`.
