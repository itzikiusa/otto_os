# Architecture review — quality loop 2026-10-21, iteration 1

Lens: structure — god files/functions that keep growing, wrong dependency
direction, leaky abstractions, duplicated subsystems, otto-server holding domain
logic, UI modules reaching into each other's internals.

Method: read-only (`git log --since=3.weeks --stat`, `grep`, `sed`, `wc`); no
builds/tests. Every finding below was confirmed by reading the cited code.
`ui/src/modules/rooms/games/**` excluded.

## Score

Rubric: 10.0 − 1.0·blocker − 0.3·major − 0.1·minor − 0.03·nit.

| Severity | Count | Deduction |
|----------|-------|-----------|
| blocker  | 0     | 0.00 |
| major    | 10    | 3.00 |
| minor    | 7     | 0.70 |
| nit      | 0     | 0.00 |

**Score: 10.0 − 3.0 − 0.7 = 6.3 / 10**

## Previous findings

None — first architecture pass of this loop.

## Areas covered

- 3-week churn ranking of `crates/` and `ui/` (git log --stat).
- Internal crate dependency graph (every `crates/*/Cargo.toml [dependencies]`).
- otto-server: `modules.rs`, `workflow_engine.rs` (execute_node arms),
  `workflow_api_request.rs`, `routes/api_client.rs` (+ api_* siblings),
  `skill_review.rs`, `skill_eval.rs`, `vault_docs_agent.rs`, `product_host.rs`,
  `policy.rs` (read, no finding), `rooms/` vs `game_rooms/` (read, no finding —
  deliberately isolated), `state.rs` ServerCtx.
- otto-telemetry / otto-usage / otto-git / otto-dbviewer / otto-agent-run
  dependency chain; otto-core `cancel.rs`; otto-swarm `runtime/run.rs`.
- ottod: `mcp_tools.rs` vs `otto-mcp/src/outward/{catalog,exec}.rs`;
  `usage_tailer.rs` vs otto-transcript/otto-usage.
- otto-sessions `manager.rs` (size trend only — shrinking, no finding).
- UI: cross-module import graph over `ui/src/modules`, `lib/` → `modules/`
  imports, markdown/mermaid renderers, `lib/api/client.ts` bypasses,
  `WorkflowsPage.svelte`, `GraphView.svelte`, `Navigator.svelte` (no finding —
  imports lib only), `lib/stores/database.svelte.ts`, `lib/api/types.ts`.

## Areas NOT covered yet (next iteration)

- otto-sessions `ws.rs` (4,015) and `manager.rs` internal cohesion (151 methods).
- otto-dbviewer `service.rs` + drivers (mongodb 4,975 / clickhouse 3,712 /
  mysql 3,449 / postgres 2,958): shared driver trait vs per-driver duplication.
- otto-product `service.rs`/`run.rs`/`http.rs`, otto-design / design-assist.
- otto-state repo files (`personal_agents.rs` 3,458, `product.rs`, `workflows.rs`).
- otto-issues `jira.rs`/`confluence.rs`; otto-rbac `tokens.rs` (3,651).
- UI: `Terminal.svelte` (3,632), `product/OverviewTab.svelte` (3,491),
  `database/*` components (ResultsGrid, QueryEditor, QueryBuilder),
  `git/ReviewPanel.svelte`/`DiffViewer.svelte`, `shell/App.svelte`,
  `lib/api/mock.ts`, `lib/stores/*` other than database.
- Contract docs (`docs/contracts/*.md`) vs routes drift.

---

## Findings

### architecture-1-01 — major — `execute_node` is a ~2,800-line function
`crates/otto-server/src/workflow_engine.rs:3786`

Evidence: `execute_node` spans lines 3786–6588 and matches on node kind inline.
Arm sizes: `review_run` 840 lines (:5096), `git_pr` 366 (:6109), `loop` 308
(:4788), `channel_notify` 178, `prepare_context` 142, `human_approval` 138,
`product_*` 113, `self_improve` 113. Two arms already prove the extraction pattern:
`"api_run" => api_request::execute(ctx, ws, user, p)` and
`"product_publish" => product_publication::execute(...)`. The file is the
workflow hot spot (44 commits in 3 weeks). `run_workflow_with_budget`
(:1801–3212) is a second ~1,400-line function in the same file.

Cost: every node-kind change edits the same function; arms share dozens of
locals so a change in one can affect another; per-arm unit tests are impossible
without building the whole `RunEnv`.

Fix: add `workflow_engine/nodes/<kind>.rs`, each with
`pub(super) async fn execute(call: NodeCall<'_>) -> Result<(Value, Vec<String>)>`,
where `NodeCall` bundles `ctx, ws, user, node, input, env, scope, session_tx,
log_tx, …`. Move `review_run`, `git_pr` and `loop` first. Leave `execute_node`
as a dispatch table. Split `run_workflow_with_budget` into
admission/schedule-loop/finalize phases the same way.

### architecture-1-02 — major — `modules.rs` has become the review/drafting/session-input service for the whole crate
`crates/otto-server/src/modules.rs:1` (7,676 lines, 46 commits in 3 weeks)

Evidence: the file's own doc says it is the "integration seam" for ctx-trait
impls (lines 190–907, ~25 `impl X for ServerCtx`). Over time it also took in:
- orchestrator routes (:985);
- the review run engine: `run_review_core` (:1911–2337),
  `summarize_and_persist` (:2397–2847), `retry_review_agent` (:4383),
  `retry_summarizer` (:4596), `stop_review_agent` (:4833);
- review config, readiness (:3441–3662), and PR/commit drafting (:3893–4352);
- session input routes (:6061–6366);
- a browser proxy with a ticket store (:6366–6690), although
  `routes/browser.rs` exists;
- DB explain (:6869), inject-session (:7009) and attach-product (:7124).

Other files use it as a service façade: `crate::modules::` is referenced 40+
times from 8 files (`workflow_engine.rs`, `run_engine.rs`, `run_service.rs`,
`vault_docs_agent.rs`, `db_assist.rs`, `automation_ctx.rs`, `routes/proof.rs`,
`boot/build.rs`). Those references include `cancel_running_review` (×6),
`resolve_skill_inline` (×5), `StagedSkillPackages` (×5) and
`default_review_provider` (×4). `pub use otto_review::engine::*;` (:38) is a glob
re-export shim "so the `crate::modules::…` paths other engines use keep
resolving".

Cost: unrelated features (browser proxy, session typing, PR drafting, reviews)
all merge into one file. The glob re-export hides where review symbols really
live. Skill staging, which several engines share, is only reachable through
"modules".

Fix: split by concern:
- `ctx_impls.rs`
- `review/{routes,run,retry,summarize,lifecycle}.rs`, exposing a small
  `review::service` API for workflow/run engines in place of `crate::modules::*`
- `pr_draft.rs`
- `session_input.rs`
- `routes/browser_proxy.rs`
- `skill_staging.rs`, or `otto-skills` for the pure part

Replace the glob re-export with explicit `use otto_review::engine::{…}` at the
call sites.

### architecture-1-03 — major — the cancel-registry operations are copied 6× with diverging semantics
`crates/otto-core/src/cancel.rs:12`

Evidence: otto-core only provides the type alias
`pub type CancelRegistry = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>`. The doc
says it exists to avoid "three identical copies". The operations are still
re-implemented in each engine:
- `otto-server/src/skill_review.rs:58` has `// Cancel registry helpers
  (re-declared; skill_eval's are private)` and defines
  `register_cancel`/`signal_cancel`/`unregister_cancel` at :61/:68/:78.
- `skill_eval.rs:43/62/115`.
- `modules.rs:1633/1647/1660` (`*_review_agent_cancel`).
- `product_host.rs:288/300`.
- `otto-swarm/src/runtime/run.rs:31/39/45`.
- `otto-product/src/run.rs:429`.

The copies behave differently:
- **Unregistering a superseded flag:** `product_host.rs:296-311` only removes the
  flag if it is still the same one (`Arc::ptr_eq`, fix "S4-16": a quick
  Stop+Retry must not let the old loop remove the new flag).
  `ReviewCancelGuard::drop` (`modules.rs:2357`) does the same. `skill_eval.rs:115`,
  `skill_review.rs:78`, `modules.rs:1660` and `otto-swarm run.rs:45` remove the
  flag unconditionally.
- **Lock poisoning:** swarm calls `.lock().unwrap()` and panics on a poisoned
  lock; the others use `if let Ok`.
- **Retry keys:** `skill_review` also trips `{id}:retry:*` keys; `modules.rs`
  uses `{review}:{index}` keys.

Cost: a fix like S4-16 lands in one or two copies and the same bug class stays
open in the rest.

Fix: make `CancelRegistry` a newtype in otto-core with `register(id) ->
CancelGuard` (RAII, unregisters only if still `ptr_eq`), `signal(id)`,
`signal_prefix(prefix)` and `is_live(prefix)`. Poison-tolerant locking goes
inside it. Delete the six local copies.

### architecture-1-04 — major — the "N agents + summarizer" run engine is re-implemented per feature
`crates/otto-server/src/skill_review.rs:3`, `crates/otto-server/src/vault_docs_agent.rs:14`

Evidence: the files say they are copies:
- `skill_review.rs:3`: "Mirrors the code-review engine
  (`modules.rs::run_review_core`)".
- `vault_docs_agent.rs:14`: "Mirrors the `product_run` fan-out + summarizer shape
  (JoinSet, per-agent error isolation, sessions left open)".

Separate `JoinSet` fan-outs with a trailing summarizer live in `modules.rs` /
otto-review, `otto-product/src/run.rs:548,2485`, `skill_review.rs:764`,
`skill_eval.rs:1511` and `vault_docs_agent.rs:2038,3138`. Each engine identifies
the summarizer stage differently:
- code review: by position ("always the last row", `modules.rs:4633`);
- skill review: by name (`agent.name == "summarizer"`, `skill_review.rs:420`);
- vault docs: by a sentinel index (`SUM_RETRY_IDX = usize::MAX`,
  `vault_docs_agent.rs:356`).

Each engine also has its own retry-summarizer, stop-agent and recovery code.

Cost: retry, stop and recovery semantics differ by feature, and every new
multi-agent feature copies the shape again (four times so far).

Fix: put a generic fan-out runner in `otto-agent-run`, the crate documented for
agent-run primitives. It should provide:
- an agent-row state machine (pending/running/done/error/stopped);
- per-agent cancel, using finding 03's guard;
- error-isolated JoinSet execution;
- an explicit `Stage::Summarizer` instead of positional or name conventions;
- retry/stop entry points;
- a persistence/broadcast trait each feature implements.

Migrate skill_review and vault_docs first; they are the newest copies.

### architecture-1-05 — major — two hand-maintained MCP tool catalogs, plus a third copy of the REST paths
`crates/ottod/src/mcp_tools.rs:1298`, `crates/otto-mcp/src/outward/catalog.rs:530`

Evidence: `mcp_tools.rs` is 7,431 lines and grew 5,914 → 7,431 in 3 weeks. It
hand-writes native `tools/list` schemas for about 45 tools that also exist in the
governed catalog (`otto-mcp/src/outward/catalog.rs`). The schemas have already
drifted:
- native `otto_vault_read` takes `vault_id: integer` (:1345); governed
  `otto.vault_read` takes `integer|string` + `VAULT_REF_DESC` (catalog.rs:672);
- native `otto_get_pr` requires `pr_number`; governed `otto.get_pr` uses `number`
  with `pr_number` as an alias (catalog.rs:535);
- the descriptions differ.

Native dispatch hard-codes daemon REST paths (`format!("/repos/{repo}/prs")`,
mcp_tools.rs:2589) separately from the outward executor (`exec.rs:519`). Name
pairs are also kept by hand: `VAULT_REVIEW_READ_TOOLS` (:1664) and
`GOVERNED_ALIASED_BY_NATIVE`. "Native wins by name" (`governed_tools_listed`,
:1860), so for these tools a session sees the native schema while the control
plane shows the governed one. No test compares the two schemas.

Cost: the user's parity rule (control plane == session view) holds for tool
names but not for schemas. Each route rename must be made in three places.

Fix: generate native listings from `otto_mcp::outward::catalog` (`otto.x` →
`otto_x`) and resolve native calls through the outward route map (`route_for`).
Keep only stdio-only tools (UI bridge stubs, browser tab) native. Until then, add
a test asserting `inputSchema` equality for every name in both catalogs.

### architecture-1-06 — major — the low-level telemetry API depends on the embedded ClickHouse usage engine
`crates/otto-telemetry/Cargo.toml` (`otto-usage.workspace = true`), `crates/otto-telemetry/src/lib.rs:13`

Evidence: `otto-telemetry` has two parts:
- the instrumentation API, `context::measure` / `measure_result` (`context.rs`):
  a task-local plus a `service.record(span)` call;
- the ClickHouse collector (`use otto_usage::{ClickHouse, UsageEngine}`, lib.rs:13;
  schema.rs:4).

Low-level crates use only the API:
- `otto-git/src/local.rs:1171,1216,1311,1336,1356`;
- `otto-dbviewer/src/service.rs:2421,2430,2455`;
- `otto-agent-run/src/offload.rs:17`.

Through the collector, they all depend on `otto-usage` (sqlx + the embedded
ClickHouse engine). Because `otto-sessions` → `otto-git` → `otto-telemetry` →
`otto-usage`, nearly every crate depends transitively on the usage engine.

Cost: any `otto-usage` edit rebuilds otto-git, otto-dbviewer, otto-sessions and
everything above them. `scripts/check.sh` then selects all of those dependents
for nextest, on a machine where heavy parallel builds are known to freeze. The
dependency also points the wrong way: a feature crate sits under the
cross-cutting tracing layer.

Fix: split the crate in two:
- `otto-telemetry-context`: a leaf crate with `scope`/`measure`/`measure_result`
  and a `SpanSink` trait object in the task-local;
- the collector: implements `SpanSink` and depends on `otto-usage`.

Only `otto-server`/`ottod` should depend on the collector.

### architecture-1-07 — major — the REST engine lives in a 4,447-line route file, and workflows call its axum handler with fabricated extractors
`crates/otto-server/src/workflow_api_request.rs:49-66`, `crates/otto-server/src/routes/api_client.rs:2335`

Evidence: `workflow_api_request::execute` builds an
`otto_core::auth::AuthContext { … }` literal by hand (:49) and calls the HTTP
handler directly:
`crate::routes::api_client::execute(Path(ws.id.clone()), State(ctx.clone()),
CurrentUser(..), CurrentAuthContext(auth), headers, Json(req))` (:59).

`routes/api_client.rs` holds the whole request engine:
- cookie jars (:95), tunnel cache (:304);
- `prepare_request` (:3002), `build_and_send` (:3130), `apply_auth` (:3350),
  `build_multipart` (:2848), `substitute` (:4331), `resolve_dynamic_var` (:4388);
- the automation step engine (:3615–4098).

`otto-apiclient` (documented as "HTTP API-client engines split out of
otto-server") contains only `grpc.rs` and `scripts.rs`. Sibling routes reach into
the handler file's internals: `api_automation_runs.rs:103-325` calls
`super::api_client::{resolve_environment, run_step, bound_env_secret_hosts,
invalidate_storage_gauge, history_retention}`, and `api_oauth.rs:119` calls
`super::api_client::{workspace_allows_local, net_guard}`.

Cost: the engine can only be reached through an HTTP handler signature.
Internal callers must fabricate the extractors and the actor identity, so any new
extractor or auth field silently changes their behaviour. The engine can't be
tested without axum.

Fix: extract an `ApiClientService::execute(ctx, ws, actor, req)`, plus
`run_step`/`resolve_environment`, into `otto-apiclient::rest` behind an
`ApiClientCtx` trait, matching the other crates' `*Ctx` pattern. The route
handler, automations, OAuth and workflow `api_run` all call the service. Build
the actor with one constructor (e.g. `AuthContext::automation(user, "workflow")`)
instead of a struct literal.

### architecture-1-08 — major — `WorkflowsPage.svelte` is a 5,145-line, still-growing god component
`ui/src/modules/workflows/WorkflowsPage.svelte:1`

Evidence: the file grew 4,625 → 5,145 lines in 3 weeks (20+ commits). It has
1,886 script lines, 98 functions and 36 `$state` declarations, covering:
- workflow list, row menus and CRUD (`load`, `del`, `duplicate`, `commitRename`;
  :623–940);
- graph editing (`addNode`, `removeSelected`, `tidy`);
- run lifecycle (`startRun`, `followRun`, `waitRunTerminal`, `refetchRun`,
  `stop`; :266–1073);
- approvals (`approveRun`, `approvalGate`);
- a per-node-kind inspector with bespoke forms: reviewers (:1653–1693), loop
  steps (:1738–1779), self-improve (:1787), retry policy (:1798), edge conditions
  (:1830);
- version restore (:1848–1870);
- a hand-rolled markdown preview (:373).

The module's small helpers (`retryPolicy.ts`, `runInput.ts`, `triggerForm.ts`)
show the extraction direction, but the page keeps growing.

Cost: each new node kind adds inspector state and functions to this file. That
mirrors finding 01 on the server.

Fix: extract
- `NodeInspector.svelte` with per-kind sub-forms (`ReviewRunForm`,
  `LoopStepsForm`, `SelfImproveForm`, `RetryForm`), keyed by node kind;
- a `workflowRun.svelte.ts` controller for start/follow/stop/approve;
- `WorkflowList.svelte`.

The page then composes them.

### architecture-1-09 — major — `GraphView.svelte` mixes layout math, ref sidebar and every git operation
`ui/src/modules/git/GraphView.svelte:1`

Evidence: the file grew 4,898 → 5,320 lines in 3 weeks. Its script block alone
is 3,169 lines with 125 functions:
- ref sidebar with folders and drag/drop (`groupBranches`, `toggleFolder`,
  `onRefDragStart`…`onTargetDrop`);
- graph paging and virtualization (`loadPage`, `syncViewport`,
  `onGraphScrollFrame`, `flushPending`);
- pure lane layout (`allocateLane`, `laneColorAt`, `freshColor`, `branchOf`);
- one menu and action set per object type (`commitMenu`, `branchMenu`,
  `tagMenu`, `stashMenu`, `wtMenu`, `subMenu`, plus `cherryPick`,
  `revertCommit`, `rebaseOnto`, `deleteRemoteBranch`, …);
- diff scheduling (`scheduleDiff`, `startDiff`, `holdDiff`);
- the new landed-in marks (`landedIn`, `setCleanupBase`).

Cost: the pure lane allocator can't be unit-tested on its own. Every new git
action or ref feature lands in the same component, which already re-renders a
virtualized list.

Fix: extract
- `graph-lanes.ts`: pure lane allocation and colours, with unit tests;
- `ref-menus.ts`: builds the menus and dispatches the git actions through
  `gitBridge`;
- `RefSidebar.svelte`: branches/tags/stashes/worktrees/submodules plus
  drag/drop.

GraphView keeps only the virtualized graph and selection.

### architecture-1-10 — major — `DatabaseStore` is a 4,348-line singleton spanning a dozen sub-domains
`ui/src/lib/stores/database.svelte.ts:4332` (`export const database = new DatabaseStore()`)

Evidence: the file grew 3,939 → 4,348 lines across 22 commits in 3 weeks. It has
about 128 members covering:
- AI assist (:969–1158);
- query tabs and drafts (:1278–1603);
- connection lifecycle, warm-up and keep-alive (:1974–2461);
- SSH tabs (:2466–2482) and Kafka focus (:2461);
- the schema tree and object search (:2694–2983);
- query execution, paging, explain and abort/reattach (:3039–3547);
- Redis reads (:3658–3695);
- filter chips (:3783–3859);
- completions (:3892), saved queries (:3943–4065), history (:4083–4123);
- dashboards and widgets (:4145–4295).

It also imports 9 helpers from `modules/database/*` (:51–67), so lib depends on
the feature module that depends on it.

Cost: every DB-explorer feature touches one class, and its reactive state is all
on one object, so unrelated features can invalidate each other's state.

Fix: move the store into `modules/database/store/` as composed sub-stores:
`connections`, `tabs`, `schemaTree`, `execution`, `savedQueries`/`history`,
`dashboards` and `assist`. Keep a thin `database` façade so call sites move in
stages.

### architecture-1-11 — minor — shared renderers live inside feature modules, and markdown is rendered five different ways
`ui/src/modules/canvas/mermaid*`, `ui/src/modules/vault/mdRender.ts:1`, `ui/src/modules/database/JsonTree.svelte`

Evidence (cross-module import scan):
- `canvas/mermaid` is imported by product (7), design-hall (4), workbench (4),
  vault (3) and panels.
- `vault/mdRender` is imported by `agents/conversation/chatMarkdown.ts:13`,
  `browser/ReaderView.svelte:24`, `panels/OutputsPanel.svelte:21` and
  `workbench/preview/MarkdownPreview.svelte:7`.
- `database/JsonTree.svelte` is imported 7× by aws (`Ec2View.svelte:23`, …).

Next to `lib/md.ts` `renderMarkdownGfm` (`breaks: false` + sanitizer), four
components call `marked` directly with `breaks: true`:
`panels/FileTree.svelte:334` (no sanitizer; iframe-only),
`workflows/WorkflowsPage.svelte:373` (no sanitizer; iframe-only),
`api/RequestBuilder.svelte:345` and
`kubernetes/monitor/MonitorInsights.svelte:44`.

Cost: the same markdown renders differently on different surfaces. Every new
call site has to remember to sanitize. Feature modules end up owning shared
infrastructure.

Fix: move these into `lib/render/` (`mermaid.ts`, `markdown.ts` with
`renderMarkdownGfm`/`renderVaultMarkdown` presets) and
`lib/components/JsonTree.svelte`. Route the four direct `marked` calls through
`lib/md`.

### architecture-1-12 — minor — product ↔ design-hall and product ↔ swarm import cycles; the 3D engine is owned by the wrong module
`ui/src/modules/design-hall/studio3d/embed.ts:38-45`, `ui/src/modules/product/MockupsTab.svelte:7`

Evidence: design-hall imports `product/design/*` internals 18 times:
- `scene3d/{build,environment,validate,types,tokens,presets,states,optimize}`
  from `studio3d/embed.ts`, `Studio3D.svelte:58-61` and `StatesBar.svelte:19`;
- `DeviceFrame`/`DesignBoard` from `ArtifactStage.svelte:16-17` and
  `ArtifactView.svelte:51`;
- `templates`/`format` from `create.ts:8-10`.

Product imports design-hall back (`MockupsTab.svelte:7` → `DesignGraphStrip`). The
token grammar is copied instead of shared: `product/design/scene3d/validate.ts:65-66`
says "Brand Kit's token-name grammar (`design-hall/brand/tokens.ts::isValidTokenName`)"
and re-declares the regex. Types also form a cycle: `product/types.ts:7` imports
`../swarm/types`, and `swarm/types.ts:377` imports `../product/types`.

Cost: neither module can change its internals without breaking the other. The
copied grammar will drift.

Fix: move `scene3d/`, `DeviceFrame`, `DesignBoard` and the token grammar into a
shared `lib/design/` that both modules import. Move the shared
Product/Swarm contract types into `lib/api/types` (see finding 13).

### architecture-1-13 — minor — `lib/` (the shared layer) imports feature modules, including the API contract mirror
`ui/src/lib/api/types.ts:12095-12098`, `ui/src/lib/router.svelte.ts:18-19`

Evidence: 34 imports go from `lib/` into `modules/`:
- `lib/api/types.ts` re-exports Product publication/transcript contract types
  "owned by Product" from `../../modules/product/types`.
- `lib/router.svelte.ts` imports `modules/rooms/room-access` and
  `modules/rooms/games/access`.
- `lib/stores/{canvas,apiClient,product,swarm,k8s,assistant,mockup-assist}`
  import module internals.
- `lib/uiCommands/{k8s,browser,vault,swarm,brokers,git,home}.ts` import
  module internals.

Finding 10 counts `lib/stores/database` separately.

Cost: AGENTS.md says `ui/src/lib/api/types.ts` mirrors the Rust contract, but
part of the contract now lives in feature modules. With no layering, cycles go
unnoticed.

Fix: move contract types into `lib/api/types`. Move pure helpers that lib needs
(`k8s-util`, `viewState`, `graphqlVars`, `room-access`) into `lib/`, or move
module-specific stores and ui-commands into their modules. Add a
`scripts/ui-guards.mjs` rule that forbids new `lib/` → `modules/` imports
(ratcheted, like the other guards).

### architecture-1-14 — minor — four call sites bypass the API client with hand-rolled authed `fetch`
`ui/src/modules/product/MockupViewer.svelte:74-79`, `ui/src/modules/canvas/canvasFiles.ts:25-29`, `ui/src/modules/design-hall/brand/api.ts:34-45`, `ui/src/modules/design-hall/site/siteApi.ts:28`

Evidence: each call site builds `Authorization: Bearer ${getToken()}` and calls
`fetch(`${baseUrl()}/api/v1/...`)` itself; `brand/api.ts` also re-implements
Problem parsing. The client's private `rawRequest` (`lib/api/client.ts:407`)
does more than that:
- routes through `laneFetch` (alt-loopback lane fallback);
- reports provider health;
- dispatches `UNAUTHORIZED_EVENT` on 401;
- throws a typed `ApiError`.

Cost: a 401 on these paths doesn't trigger re-auth, they ignore the lane
fallback, and their errors aren't `ApiError`s.

Fix: export `authedResponse(path, init)` from `client.ts`, a thin wrapper over
`rawRequest`, or use the existing `authedText`/`authedBlobUrl`. Replace the four
copies.

### architecture-1-15 — minor — review status transitions are hand-rolled at every exit path
`crates/otto-server/src/modules.rs:4709-4790` (and 14 other sites)

Evidence: `Event::ReviewChanged { workspace_id, session_id: None, review_id,
status }` is built by hand 15 times in `modules.rs` and twice in
`review_launch.rs`, usually right after a `reviews_store.set_status(...)` call.
`retry_summarizer` alone does this pair 3 times (the worktree error, the
summarize error and the final status). `skill_review.rs:865` already has a
local `emit` helper for its own engine.

Cost: a new exit path that forgets the broadcast leaves the UI showing a stale
"running" review.

Fix: add `review::lifecycle::transition(ctx, ws_id, review_id, status,
error)`. It persists and broadcasts in one call; use it at every site. Folds
into finding 04 when that engine is shared.

### architecture-1-16 — minor — the usage ingestion driver lives in the `ottod` binary, and Codex model extraction is triplicated
`crates/ottod/src/usage_tailer.rs:1` (2,388 lines + `usage_tailer/native.rs` 404), `:1069-1083`

Evidence: transcript discovery, FSEvents watching, attribution and the rebuild
logic all live in the binary crate. The parsing core and ingestion journal are
already in `otto-transcript::usage` / `otto-usage::ingestion`. The
Codex-model-from-JSON logic now exists three times with different fallbacks:
- `usage_tailer.rs:1073-1081`: `payload.model` as a string, or `.name` / `.id` /
  `.slug`, for `turn_context` | `session_meta`;
- `otto-transcript/src/usage.rs:503-514`: `payload.model` or a top-level
  `model`, `session_meta` only;
- `otto-transcript/src/codex.rs:1074`.

Cost: model attribution can differ between the usage store and the
conversation view. The tailer can only be tested inside the binary crate.

Fix: move the tailer driver into `otto-usage` (e.g. `otto_usage::tailer_driver`).
Add one `otto_transcript::codex::model_of(&Value)` used by all three.

### architecture-1-17 — minor — `lib/api/types.ts` is a single 12,171-line merge hotspot
`ui/src/lib/api/types.ts:1`

Evidence: it is the most-changed file in the repo, with 113 commits in 3 weeks.
It grew 10,696 → 12,171 lines, and every feature appends to the same file.
Several agents edit the repo in parallel, so each feature branch rebases through
this file.

Fix: split it per contract area into `lib/api/types/<area>.ts` (sessions, git,
db, workflows, product, mcp, …), matching `docs/contracts/` sections. Keep
`lib/api/types.ts` as a barrel `export *` so imports don't change.
