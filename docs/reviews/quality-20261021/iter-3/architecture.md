# Architecture review: quality loop 2026-10-21, iteration 3

Lens: structure. That covers god files and functions that keep growing, dependencies that point the
wrong way, leaky abstractions, duplicated subsystems that should be shared, domain logic held in
otto-server, and UI modules reaching into each other's internals.

The review was read-only, using `git log`, `rg`, `sed`, `awk`, `wc` and one local Python import of
`scripts/loc-ratchet.py` to count production lines. Nothing was built or tested. Every finding was
confirmed against `fix/quality-loop-20261021` at `3dd55d9f2`.

These paths were excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ClassroomsBox.svelte`, `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Earlier findings still open, fully or as a residual | 0 | 4 (1-04, 1-08, 1-09, 1-10) | 10 (1-01r, 1-05, 1-07r, 1-12, 1-13, 1-16, 2-01r, 2-05, 2-06, 2-08) | 1 (2-03r) |
| New in iteration 3 | 0 | 2 (3-01, 3-02) | 4 (3-03 … 3-06) | 0 |
| **Total** | 0 | 6 | 14 | 1 |

**Score: 10.0 − 6 × 0.3 − 14 × 0.1 − 1 × 0.03 = 10.0 − 1.8 − 1.4 − 0.03 = 6.77 / 10**

The score went up from 5.90 to 6.77. Iteration 2 fully fixed 10 earlier findings: 1-02, 1-06, 1-11,
1-15, 1-17, 2-02, 2-04 and 2-07, plus the cycle half of 1-12 and the fabricated-extractor half of 1-07.
It took the worst god function, `execute_node`, from about 2,830 lines to 206. It also added
production-line and per-function ratchets, so the remaining hot spots can no longer grow unnoticed.

To reach 9.8, the work that remains is:

- the three UI god files (1-08, 1-09, 1-10);
- the fan-out runner (1-04);
- the two new majors. Both are cheap and fix real drift:
  - 3-01 swarm activation;
  - 3-02 the Atlassian transport.

## Status of earlier findings

| ID | Sev (then → now) | Status | Evidence now |
|----|------------------|--------|--------------|
| 1-01 `execute_node` / `run_workflow_with_budget` | major → **minor (residual)** | Partly fixed | `execute_node` is now a 206-line dispatch (`workflow_engine.rs:3680–3885`). The 17 node kinds live in `workflow_engine/nodes/*.rs`, and the largest is `review_run.rs` at 878 lines. The catalog in `otto-workflows/src/catalog.rs` and the dispatch arms list the same 26 kinds, which I checked by diffing the two. **Still open:** `run_workflow_with_budget` spans `workflow_engine.rs:1850–3102`, about 1,252 lines. Its admission and schedule phases are not split (FIXES P01). The `functions.run_workflow_with_budget: 1253` ratchet caps it, so this is re-rated minor. |
| 1-02 `modules.rs` façade | major | **Fixed** | `modules.rs` is 1,089 lines. The glob re-export is replaced by named imports (`:42–53`). Review lives in `modules/review/{lifecycle,run,summarize,retry,routes}.rs`. `browser_proxy_ticket` (`:652`) is the only documented leftover, and it is trivial. |
| 1-04 fan-out + summarizer engine per feature | major | Open (deferred) | Each engine still identifies the summarizer its own way: `skill_review.rs:400` checks `agent.name == "summarizer"`, `vault_docs_agent.rs:356` uses `SUM_RETRY_IDX = usize::MAX`, and `modules/review/summarize.rs:56` uses `summarizer_idx`. `otto-agent-run/src` has no fan-out runner (`agent_run.rs`, `offload.rs`, `screen.rs`, `turn_oracle.rs`). |
| 1-05 two MCP catalogs | minor (residual) | Open (deferred) | `ottod/src/mcp_tools/tests/catalog_tests.rs:16`: `DEFERRED_SCHEMA_DRIFT` still lists 7 `otto_api_*` and `otto_list_connections` pairs. `mcp_tools.rs` is 7,394 lines. |
| 1-06 telemetry → usage; sessions → git | major | **Fixed** | `otto-telemetry` has no internal dependencies. `otto-telemetry-collector` depends on `otto-usage`. `otto-sessions` now depends on the leaf `otto-git-cmd`, not `otto-git`. I re-derived this from every `Cargo.toml`. |
| 1-07 REST engine in a route file; fabricated extractors | major → **minor (residual)** | Partly fixed | Fabricated extractors are gone. The only remaining `State(ctx.clone())` and `CurrentUser(..)` handler calls are in `#[cfg(test)]` blocks (`routes/settings.rs:459`, `skill_eval.rs:3543`, `share.rs:1089`, `capabilities.rs:762`, `email_sender.rs:220`). **Still open:** the API-client execution engine is `routes/api_client.rs` (3,266 lines; `execute` `:1808`, `resolve_environment` `:2017`, `run_step` in `routes/api_client/automations.rs:137`). Its 1,770-line pure helper module `api_helpers.rs` (curl parser, OpenAPI export, assertion evaluator; no `ServerCtx`) is still in otto-server. Non-route code calls into the route module: `workflow_api_request.rs:49` → `crate::routes::api_client::execute_service`, and `routes/grpc.rs:49,81` → `routes::api_client::workspace_allows_local`. `otto-apiclient` still holds only gRPC and scripts. |
| 1-08 `WorkflowsPage.svelte` | major | Open | 5,156 lines. Now capped by the `file-loc` ratchet (`ui-guards-baseline.json`). |
| 1-09 `GraphView.svelte` | major | Partly fixed, still major | 5,320 → 4,878 lines (`graph-lanes.ts` and `ref-menus.ts` extracted). RefSidebar and every git action are still inline. |
| 1-10 `DatabaseStore` | major | Open, and it grew | 4,383 → **4,415** lines during iteration 2. It is now capped at 4,415. |
| 1-11 shared renderers in feature modules | minor | **Fixed** | `marked` is imported only by `lib/md.ts`, `lib/render/markdown.ts` and `agents/conversation/chatMarkdown.ts` (the last is a type import over `renderNote`). `canvas/mermaid.ts` and `vault/mdRender.ts` are now 3-line re-export shims. |
| 1-12 product ↔ design-hall / swarm cycles | minor | Partly fixed, still open | The product ↔ swarm type cycle is gone (`lib/api/types/`). The product ↔ design-hall cycle remains: `design-hall/ArtifactStage.svelte:17` imports `product/design/DesignBoard.svelte`, `design-hall/create.ts:8,10` imports `product/design/{templates,format}`, and `product/MockupsTab.svelte:7` imports `design-hall/DesignGraphStrip.svelte`. |
| 1-13 `lib/` imports `modules/` | minor | Open, and it grew | There are now **36** imports in 17 files (was 34). The JsonTree move added 3 of them: `lib/components/JsonTree.svelte:30–32` imports `modules/database/{bson,expansion-plan,json-find}`. So the "shared" component still depends on the feature it was moved out of. The `lib-imports-modules` ratchet caps the count at 36. |
| 1-15 `ReviewChanged` built by hand | minor | **Fixed** | `modules/review/lifecycle.rs:33` is the single review-level emitter. `summarize.rs:175` is an agent-session update, and `run_engine.rs:1037` is a test. |
| 1-16 usage tailer in the binary; Codex model triplicated | minor | Open | `ottod/src/usage_tailer.rs` is 2,424 lines. `"slug"` fallbacks are still at `usage_tailer.rs:1111`, `otto-transcript/src/usage.rs:512` and `codex.rs:1074` (FIXES P09: not done). |
| 1-17 `types.ts` monolith | minor | **Fixed** | `lib/api/types.ts` is a 59-line `export *` barrel over 42 area files. |
| 2-01 terminal protocol decoded twice | major → **minor (residual)** | Partly fixed | `termProtocol.ts` (123 lines) provides `decodeTermFrame` and `applySnapshot`, used by both `wireParked` (`Terminal.svelte:170,177`) and `wireSocket` (`:1036`). **Still open:** the resize and claim negotiation (`deferLocalResize` `:1108`, `claimGrid` `:1338`, `sendResize` `:1381`, `applyGrid` `:1439`, `safeFit` `:1459`) remains in the 3,608-line, 85-function component. `termGridSync.ts` does not exist. |
| 2-02 SQL classification ×4 | minor | **Fixed** | `otto-dbviewer/src/sql_class.rs` holds one scanner and `ReadClass::of`. The drivers' `is_read_statement` functions are one-line delegates (`postgres.rs:1606`, `mysql.rs:2025`). `drivers/sql_common.rs` holds `float_to_json` and `acquire_error`. |
| 2-03 workspace store absorbing feature state | minor → **nit (residual)** | Mostly fixed | Settings are typed, and workflow runs moved to `workflowRuns.svelte.ts`. `ws` keeps only delegates (`workspace.svelte.ts:152,470,476`). The residual is three API-client getters on `ws` (`apiAllowLocal` `:2097`, `apiHistoryRetention` `:2111`, `apiRunsKeep` `:2129`). |
| 2-04 hand-written event chain | minor | **Fixed** | `lib/events/routes/*.ts` (12 route files) has an exhaustiveness test, and `events.svelte.ts` has no `parsed.type ===` chain left. |
| 2-05 shell root owns palette sources | minor | Open | `shell/App.svelte` still has 49 `$effect(` and 14 `registry.register(`, including the connections source that POSTs `/connections/{id}/open` (`:923`). |
| 2-06 rooms inside otto-server | minor | Open (deferred to after the games team merges) | `rooms/` and `game_rooms/` are unchanged. |
| 2-07 LOC guard counts the wrong lines; no UI guard | minor | **Fixed** | `loc-ratchet.py:44–66` counts production lines only and caps `execute_node`, `run_review_core` and `run_workflow_with_budget`. `ui-guards.mjs` adds `file-loc` (104 files baselined) and `lib-imports-modules` (36). |
| 2-08 game map ids hand-synced | minor | Open (deferred) | `game_rooms/types.rs:11–23` is unchanged. |

No earlier architecture finding was rejected.

## Areas covered in this iteration

- **Earlier findings.** Re-verified all 23 earlier findings against the code, as listed above. This
  included diffing the workflow catalog kinds against the `execute_node` dispatch arms (26 = 26).
- **Crate graph.** Re-derived the internal crate graph from every `crates/*/Cargo.toml`, including the
  new `otto-git-cmd`, `otto-telemetry` and `otto-telemetry-collector` crates. Checked the product → swarm,
  product → improve and design-assist → improve edges. Only product → swarm produced a finding
  (3-01). The improve edges use the engine's public evidence API.
- **otto-product.** Covered `swarm.rs` (discovery launch), `agent_attempt.rs`, `analysis.rs`, the
  structure of `run.rs` and its function sizes, and the `ProductStudioHost` swarm primitives.
- **otto-swarm.** Covered `runtime/engine.rs`: the lifecycle handlers `start`, `pause`, `abort` and
  `resume`, the `operation_guard` visibility, and `start_coordinator`. Also covered the channel launch
  path in `runtime/channels.rs`.
- **otto-issues.** Covered the transport layers of `jira.rs`, `confluence.rs` and `linear.rs`, and the
  account validation in `http.rs` (the area iteration 2 listed as uncovered).
- **otto-server.**
  - `api_helpers.rs`, `mcp_outward.rs` (function sizes; `governed_invoke` is 558 lines but cohesive)
    and `ui_bridge.rs` (cohesive, no finding).
  - The `routes/usage.rs` budget functions and their 8 external callers.
  - The `routes/transcript.rs` helpers used outside routes.
  - The size of `lib.rs` and the `ServerCtx` field count.
  - Placement of the skill-eval and skill-review subsystem (about 7.9k production lines). This is
    noted only: no new cost beyond 1-04.
- **otto-dbviewer.** Covered `service.rs` engine branching (about 20 production `Engine::` matches,
  bounded) and the `sql_class` consolidation.
- **Shared primitive.** Swept every per-key async-lock registry in the workspace.
- **otto-core.** Covered the production-line size of `api.rs` against `new_file_max`.
- **UI.**
  - The module-to-module import matrix (excluding the games and school paths).
  - The OverviewTab Jira editor and all `/issue/*` callers.
  - Shell imports, and `apiClient.svelte.ts` structure (cohesive single-feature store, capped).
  - Where WorkflowsPage's node-kind lists come from, and AWS's reuse of `database/ResultsGrid`.
  - The size and API use of the agents session views.

## Areas NOT covered yet (next iteration)

- **otto-design and otto-design-assist.** `otto-design` `store.rs` (3,170) and `service.rs` (2,945);
  `otto-design-assist` `lib.rs` (3,290) internals.
- **Other large backend files.**
  - `otto-usage` `engine.rs` (3,291).
  - `otto-dbviewer` `drivers/mongodb.rs` (4,975, ratcheted) internals.
  - `otto-sessions` `ws.rs` (3,755) since iteration 2, and `otto-rbac` `tokens.rs`.
  - `otto-server` `routes/workflows.rs` (2,575) against `otto-workflows`.
  - `otto-server` `policy.rs` (38 commits in 3 weeks): iteration 2 judged it central by design, and it
    was not re-examined.
- **Near the 4,000-line cap.** Production lines of untracked files approaching `new_file_max`:
  - `otto-dbviewer/service.rs` (3,587);
  - `skill_eval.rs` (3,364);
  - `otto-swarm` `engine.rs` (3,269).
- **UI.**
  - `git/DiffViewer.svelte` (2,343), `git/WipPanel.svelte`, `database/QueryEditor.svelte` (2,230) and
    `database/QueryBuilder.svelte` (2,159).
  - `shell/Navigator.svelte` (2,515) internals and `scheduled-tasks/ScheduledTasksPage.svelte`.
  - `modules/proof/ProofPage.svelte`.
  - Shared components living in feature modules: `home/LazyNeedsYouLink`, used by mcp, assistant and
    mission-control, and `agents/SessionView`, embedded by loops and swarm.
- **Outside the crates and UI.** The desktop shell (`apps/desktop/src-tauri`) and
  `examples/plugins/**`.

---

## New findings

### architecture-3-01: major: swarm activation is re-implemented in four places, and the otto-product copy cannot take the lifecycle lock

**Location:** `crates/otto-product/src/swarm.rs:855-873`, `crates/otto-swarm/src/runtime/engine.rs:35`, `:2533-2561`, `:2625-2652`, `crates/otto-swarm/src/runtime/channels.rs:136-152`

**Evidence**

"Make a swarm active" is a fixed sequence:

1. take the per-swarm `operation_guard`;
2. apply the gates;
3. run `set_swarm_status(.., "active")`;
4. clear the in-memory pause;
5. call `start_coordinator`;
6. call `emit_status`.

That sequence is written out by hand four times, and the copies already differ.

| Copy | Lock | Gates | Notes |
|------|------|-------|-------|
| `engine.rs::start` (`:2533`) | `operation_guard` | `budget_blocked` | — |
| `engine.rs::resume` (`:2625`) | `operation_guard` | `budget_blocked` | Also calls `set_paused(false)`. |
| `channels.rs:136-152` | `operation_guard` | `channel_start_refused` (aborted, pause reason, budget) | Its comment records the bug that the duplication already caused: "a channel message used to flip an ABORTED or budget-paused swarm back to active … and spend past the cap". |
| `otto-product/src/swarm.rs:855-873` (`discover_story`) | **none** | `ctx.usage_budget(ws, "")` | Then `set_swarm_status`, `start_swarm_coordinator` and `emit_swarm_status`. Its own comment says "Replicates the `start` handler in otto_swarm::runtime::engine". |

The product copy cannot take the lock because `operation_guard` is `pub(crate)` (`engine.rs:35`).
`ProductStudioHost` only exposes the three primitives (`otto-product/src/host.rs:142-144`, implemented
in `otto-server/src/product_host.rs:517-522`).

`discover_story` targets a user-chosen existing swarm (`resolve_swarm(.., req.swarm_id)`, `:766`). It
can therefore interleave with `abort`, which holds the guard while it runs:

1. `abort` sets the status to aborted, calls `stop_coordinator` and kills the sessions
   (`engine.rs:2590-2618`).
2. Discovery's unguarded `set_swarm_status("active")` and `start_coordinator` land between those
   steps.

The result is either a swarm marked "active" whose sessions were just killed, or a fresh coordinator
that `abort` then removes while the row says "active".

**Cost**

Every new entry point that starts a swarm (product discovery, channels, and the next trigger) has to
rediscover the lock, the gates and the pause handling. The two non-route copies have each missed part
of it. A fix to the start rules, such as the channel abort and pause-reason refusal, does not reach the
other copies.

**Fix**

1. Add `otto_swarm::runtime::lifecycle::activate(ctx: &SwarmRt, ws: &Id, sid: &Id, policy: ActivatePolicy) -> Result<Swarm>`.
   It owns the guard, the gate (`ActivatePolicy::{UserStart, Resume, Channel}` selects
   `budget_blocked` or `channel_start_refused`), the status write, `set_paused(false)`,
   `start_coordinator` and `emit_status`. Move `pause` and `abort` beside it.
2. Make the engine handlers and `channels.rs` thin callers.
3. Replace `ProductStudioHost::{start_swarm_coordinator, emit_swarm_status}` and the direct
   `set_swarm_status` with one `activate_swarm(ws, sid)` host method that otto-server implements by
   calling `lifecycle::activate(.., UserStart)`.
4. Add a unit test that activates while an abort holds the guard.

### architecture-3-02: major: Jira and Confluence each keep their own copy of the Atlassian HTTP layer, and the TLS guard was added to only one

**Location:** `crates/otto-issues/src/jira.rs:122-141`, `:358-369`, `crates/otto-issues/src/confluence.rs:22-41`, `:114-133`, `crates/otto-issues/src/linear.rs:63-74`

**Evidence**

Jira and Confluence authenticate with the same Atlassian credential (`email:token` as HTTP Basic,
built identically at `jira.rs:360-361` and `confluence.rs:120-121`). They come from the same
`IssueAccount` (`http.rs:521-535`, `tracker.rs:42`). Each client keeps its own copy of the transport:

- **Client cache.** `get_or_build_client` is copied byte for byte, including the poisoned-lock
  fallback (`jira.rs:122-141`, `confluence.rs:22-41`). Linear has a third variant (`linear.rs:63-74`).
  Both Atlassian caches are keyed by `base_url + auth_header` and never pruned.
- **Error mapping.** Status checks are hand-written per call: 22 `is_success()` sites in `jira.rs` and
  6 in `confluence.rs`. Authorization headers are set by hand at 23 and 7 sites.
- **TLS guard.** Confluence refuses to send the Basic header over cleartext:
  `transport_err = otto_netguard::require_tls_or_loopback(&site_base)` (`confluence.rs:127`), checked
  by `ensure_tls()` before every request. `JiraClient::new` (`jira.rs:358-369`) has no such check, and
  `rg 'require_tls|netguard' jira.rs` finds nothing.

`create_account` only checks that `base_url` is non-empty (`http.rs:196-198`). As a result, one
`http://` Atlassian account is refused by every Confluence call but sends the same email and API token
in cleartext on every Jira call.

**Cost**

Each transport rule has to be added twice: TLS, redirects, retry/backoff, rate-limit handling, cache
eviction and error text. The shipped code shows that this does not happen. The divergence leaks a
credential, which is the highest-impact kind of drift.

**Fix**

Add `otto-issues/src/atlassian.rs` with an `AtlassianTransport { base, auth_header, http, transport_err }`.
It should own:

- a bounded or weak client cache;
- the TLS check, performed once in the constructor and enforced in a single `send(req) -> Result<Value>`;
- status-to-`Error::Upstream` mapping with the body snippet.

`JiraClient` and `ConfluenceClient` wrap it and keep only their path builders and parsers. Linear's
client can reuse the cache helper. Add a test that asserts `JiraClient` refuses `http://example.com`
like Confluence does.

### architecture-3-03: minor: route files double as the service layer for budget enforcement and transcript resolution

**Location:** `crates/otto-server/src/routes/usage.rs:465-502`, `:648-720`, `crates/otto-server/src/routes/transcript.rs`

**Evidence**

The workspace and provider spend-cap admission policy lives in an HTTP route file. That covers loading
`UsageBudgetConfig`, the cap relevance check, the window, the fold and the verdict. It is called from
8 non-route sites in 6 subsystems:

- goal loops: `routes/goal_loops.rs:88,237,424,481`;
- the workflow budget node: `workflow_engine/nodes/budget_gate.rs:23`;
- reviews: `modules/review/routes.rs:367`;
- the swarm host: `swarm_host.rs:105`;
- the product host: `product_host.rs:511`;
- the monitor: `monitor.rs:982,1003`.

To reach it, the file carries `*_pub` shims whose only purpose is to export private route internals:
`load_budgets_pub` (`:465`) and `budget_status_pub` (`:471`). Each host wraps the verdict in its own
error text ("Budget exceeded — swarm blocked" is spelled out separately in `otto-swarm` `engine.rs:2547`
and `otto-product` `swarm.rs:864`).

Transcript resolution follows the same shape. `routes/transcript.rs` is the de facto transcript service
for background code:

- `assistant_host.rs:90` (`resolve_transcript`);
- `history_index.rs:165` (`transcript_roots`);
- `agent_tasks_nudge.rs:85` (`effective_provider`);
- `transcript_tail.rs:347,750,804` (`fold_opts`, `register_work_artifact`);
- `routes/usage.rs:991` (`session_gate`).

**Cost**

Background engines (monitor, swarm, product, workflow, goal loops) depend on HTTP modules. So:

- a route-file refactor or split has to touch every engine;
- a budget-rule change has no single owner, and each caller formats the refusal itself;
- the shims grow with each new caller.

**Fix**

1. Move the budget policy to `otto-server/src/budget.rs`, alongside `workflow_budget.rs` but as a
   service: `BudgetGate::check(ctx, ws, provider) -> Result<(), BudgetBlocked>` with one `Display` for
   the refusal. `routes/usage.rs` keeps only `budgets`/`put_budgets` and calls it.
2. Move the transcript helpers to `transcript_tail.rs`'s sibling `transcript/resolve.rs`.
3. Delete the `*_pub` shims.

### architecture-3-04: minor: the per-key async lock registry is hand-rolled about 22 times, with three pruning rules and two poison policies

**Location:** `crates/otto-git/src/http.rs:521-560`, `crates/otto-swarm/src/runtime/merge.rs:38-47`, `crates/otto-swarm/src/runtime/engine.rs:35`, `:386-397`, `crates/otto-product/src/analysis.rs:220-231`, `crates/otto-product/src/agent_attempt.rs:6-16`, `crates/otto-server/src/routes/personal_agents.rs:870-881`, and others

**Evidence**

`rg 'HashMap<String, (Weak|Arc)<tokio::sync::Mutex<()>>>'` finds about 22 registries in 14 crates. They
include:

- otto-swarm: `operation_guard`, `tick_lock`, merge `branch_lock`;
- otto-git: `repo_lock`, `remote_lock`;
- otto-product: `story_start_lock`, `agent_attempt::lock`;
- otto-design: `CommitLocks`;
- otto-automation: goal-loop operations;
- otto-state: `workflow_progress` gates;
- otto-assistant: tasks;
- otto-dbviewer: gates;
- otto-aws: region single-flight;
- otto-k8s: EKS token mint and monitor flights;
- otto-browser: gates;
- otto-channels: telegram;
- otto-server: `api_secrets`, `agent_session_lifecycle`, `eval_score`, `personal_agents` and
  `backup_git`.

They differ on two properties.

**Pruning:**

- `Weak` entries with `retain(strong_count > 0)`: `agent_attempt.rs:9`, `engine.rs:35`;
- `Arc` entries with `retain(strong_count > 1)`: `engine.rs:395`, `personal_agents.rs:879`;
- never pruned: `otto-git/http.rs:528-532,553-557`, `merge.rs:41-46`, and `analysis.rs:220`, whose
  comment says "a few bytes per story ever analyzed".

**Poison:**

- `.expect("repo_locks poisoned")`, which panics: `otto-git/http.rs:528,553`;
- `.lock().unwrap()`: `merge.rs:43`;
- `unwrap_or_else(|e| e.into_inner())`: elsewhere.

Iteration 1 consolidated the same kind of copy-pasted primitive, the cancel registry, into
`otto_core::cancel` (finding 1-03). This one was not.

**Cost**

Each new serialisation point copies whichever variant is nearest. The never-pruned ones grow with every
repo, branch or story id. The panicking ones turn one poisoned lock into a panic on every later git
mutation for every repo.

**Fix**

1. Add `otto_core::keyed_lock::KeyedLocks<K>` with `lock(&K) -> OwnedMutexGuard<()>`,
   `try_lock(&K)`, weak entries pruned on insert, and poison-tolerant inner locking.
2. Replace the registries crate by crate, starting with otto-git and otto-swarm.
3. Add a one-line `ui-guards`-style `rg` check in `scripts/check.sh` that fails on a new
   `HashMap<String, Arc<tokio::sync::Mutex<()>>>`.

### architecture-3-05: minor: the UI has no issue-tracker API client, so 8 components in 4 modules hand-build `/issue/*` URLs

**Location:** `ui/src/modules/product/OverviewTab.svelte:486-1207`, `PublishDialog.svelte`, `SourceSearch.svelte`, `ImportDialog.svelte`, `ui/src/modules/agents/JiraIssuePicker.svelte`, `ui/src/modules/git/FocusView.svelte`, `FindingActions.svelte`, `ui/src/modules/settings/IssueAccounts.svelte`

**Evidence**

`lib/api/` has a typed client file for nearly every backend area (`aws.ts`, `k8s.ts`, `workflows.ts`,
`scheduledTasks.ts`, …) but none for issues.

- `/issue/accounts` is fetched separately in 5 components (JiraIssuePicker, FindingActions, FocusView,
  ImportDialog, PublishDialog, plus IssueAccounts for CRUD).
- `/issue/projects?account_id=…&meta=1` is built in 4.
- Path encoding is inconsistent:
  - `FocusView` and `JiraIssuePicker` use `encodeURIComponent(accountId)`;
  - `OverviewTab` interpolates raw values in 10 calls (`/issue/${s.account_id}/${s.source_key}/transitions`,
    `/assignee`, `/editmeta`, `/fields`, `/description`, …);
  - `PublishDialog` sends `account_id=${account}` raw.

`OverviewTab.svelte` (3,485 lines, 50 functions, 25 commits in 3 weeks) also embeds the whole Jira
issue editor: transitions, assignee, editmeta field editor, title, description, comments and dev status.
All of it is inline in the product story tab.

**Cost**

An issue-route change, or a second tracker (Linear is already in `otto-issues`), has to be made in 8
files across 4 modules, each with its own load and error handling. The only consumer of the Jira editor
is a 3.5k-line tab, so no other module (git FocusView, agents) can reuse it.

**Fix**

1. Add `lib/api/issues.ts` with typed functions such as `listAccounts()`, `projects(accountId)`,
   `issueFull(acc, key)`, `transitions(acc, key)` and `setAssignee(acc, key, user)`. Each one encodes
   its path segments.
2. Add a small cached `issueAccounts` store.
3. Move OverviewTab's editor block into `product/jira/IssueEditor.svelte` (the editmeta field editor
   alone is `beginEdit` … `saveField`).

### architecture-3-06: minor: `otto-core/src/api.rs` is 31 production lines below the new-file cap and grows about 200 lines every 3 weeks

**Location:** `crates/otto-core/src/api.rs:1`, `scripts/loc-baseline.json` (`new_file_max: 4000`)

**Evidence**

`api.rs` holds every HTTP request and response DTO for about 40 features in one file. It is 3,986 lines,
of which 3,969 are production lines by `loc-ratchet.py`'s own counter. It grew from 3,785 lines 3 weeks
ago, over 25 commits, and it is one of the most-edited contract files in the workspace.

It is not in the ratchet's `files` map, so the `new_file_max` of 4,000 applies. The next DTO addition
of about 30 lines will fail CI in an unrelated PR. The split pattern already exists:

- `otto-core/src/api/rooms.rs` and `api/recap.rs` are area files;
- iteration 2 split the UI mirror `types.ts` into 42 area files.

**Cost**

The likely response to a failing gate under time pressure is `--raise`, which turns the guard into
noise. Meanwhile 25 commits in 3 weeks are contending on one file.

**Fix**

Split `api.rs` by area along the same seams as `ui/src/lib/api/types/*.ts` (sessions, git, review,
workflows, product, swarm, dbviewer, …) into `otto-core/src/api/<area>.rs`. Keep `pub use` re-exports in
`api.rs` so no import path changes. The split is mechanical, and serde attributes travel with the
types.

---

## Carried-over findings (still open; full text in `../iter-1/architecture.md` and `../iter-2/architecture.md`)

| ID | Severity | Location | Remaining work |
|----|----------|----------|----------------|
| architecture-1-04 | major | `crates/otto-server/src/skill_review.rs:400`, `vault_docs_agent.rs:356`, `modules/review/summarize.rs:56` | Add a shared fan-out and summarizer runner in `otto-agent-run`. |
| architecture-1-08 | major | `ui/src/modules/workflows/WorkflowsPage.svelte:1` | Split the 5,156-line page: inspector forms into `workflows/inspector/<kind>.svelte`. |
| architecture-1-09 | major | `ui/src/modules/git/GraphView.svelte:1` | 4,878 lines. Extract RefSidebar and the action handlers. |
| architecture-1-10 | major | `ui/src/lib/stores/database.svelte.ts:1` | 4,415 lines, and it grew. Split it into per-concern stores once the unit harness resolves relative imports. |
| architecture-1-01 (residual) | minor | `crates/otto-server/src/workflow_engine.rs:1850` | `run_workflow_with_budget` is about 1,252 lines. Introduce a `RunState` struct, then split the admission and schedule phases. |
| architecture-1-05 | minor | `crates/ottod/src/mcp_tools/tests/catalog_tests.rs:16` | 7 drifted native/governed schema pairs. Needs a product decision. |
| architecture-1-07 (residual) | minor | `crates/otto-server/src/routes/api_client.rs:1808`, `api_helpers.rs:1`, `workflow_api_request.rs:49` | Move `execute`, `resolve_environment`, `run_step` and the pure `api_helpers.rs` into `otto-apiclient::rest`. |
| architecture-1-12 | minor | `ui/src/modules/design-hall/ArtifactStage.svelte:17`, `product/MockupsTab.svelte:7` | Product ↔ design-hall cycle. Move DesignBoard, templates and format to `lib/design/`. |
| architecture-1-13 | minor | `ui/src/lib/components/JsonTree.svelte:30-32`, plus 16 more files | 36 `lib` → `modules` imports. Move `bson`, `expansion-plan` and `json-find` with JsonTree first. |
| architecture-1-16 | minor | `crates/ottod/src/usage_tailer.rs:1111` | Move the tailer driver into otto-usage, and keep one Codex model extractor. |
| architecture-2-01 (residual) | minor | `ui/src/lib/components/Terminal.svelte:1108-1500` | Move the resize and claim negotiation to `termGridSync.ts`. |
| architecture-2-05 | minor | `ui/src/shell/App.svelte:669-940` | Move the palette sources and installers out of the app root. |
| architecture-2-06 | minor | `crates/otto-server/src/rooms/`, `game_rooms/` | Move them to an `otto-rooms` crate after the games team merges. |
| architecture-2-08 | minor | `crates/otto-server/src/game_rooms/types.rs:11` | Keep one game catalog JSON. Depends on 2-06. |
| architecture-2-03 (residual) | nit | `ui/src/lib/stores/workspace.svelte.ts:2097-2140` | Three API-client getters remain on `ws`. Move them to `apiClient` once `modules/api` stops importing `ws` for them. |
