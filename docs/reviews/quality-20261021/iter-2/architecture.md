# Architecture review: quality loop 2026-10-21, iteration 2

This lens looks at structure:

- god files and god functions that keep growing;
- dependencies that point the wrong way;
- leaky abstractions;
- duplicated subsystems that should be shared;
- domain logic held in otto-server;
- UI modules reaching into each other's internals.

The review was read-only, using `git log`, `git show`, `rg`, `sed`, `awk` and `wc`. Nothing was built
or tested. Every finding below was confirmed against the current code on `fix/quality-loop-20261021`
at `ab0a86f05`.

These areas were excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ClassroomsBox.svelte`, `target/`, `node_modules/` and `docs/reviews/`. Finding 2-08 reads
`rooms/games/shooter/catalog.ts` only to confirm that the map ids are mirrored there. It makes no
finding about that code.

## Score

Rubric: 10.0 − 1.0 per blocker − 0.3 per major − 0.1 per minor − 0.03 per nit. Only open findings
count.

| Source | Blocker | Major | Minor | Nit |
|--------|---------|-------|-------|-----|
| Iteration-1 findings still open | 0 | 8 (01, 02, 04, 06, 07, 08, 09, 10) | 7 (05 residual, 11, 12, 13, 15, 16, 17) | 0 |
| New in iteration 2 | 0 | 1 (2-01) | 7 (2-02 … 2-08) | 0 |
| **Total** | 0 | 9 | 14 | 0 |

**Score: 10.0 − 9 × 0.3 − 14 × 0.1 = 10.0 − 2.7 − 1.4 = 5.9 / 10**

The score went down from 6.3 to 5.9. Iteration 1 fixed two findings (03 and 14) and reduced a third
(05). SCORES.md deferred the other 14 architecture findings, and none of them were touched. Two of
those deferred hot spots grew during the iteration-1 fixes (see 01 and 2-07). This pass also went one
level deeper and found eight new structural problems.

The lens cannot reach 9.8 until the deferred splits are scheduled. Each one needs a single dedicated
writer. Recommended order by cost avoided: 01, 02, 2-01, 07, 06, 10.

## Status of iteration-1 findings

| ID | Sev | Status | Evidence now |
|----|-----|--------|--------------|
| 01 `execute_node` god function | major | **Open, and it grew** | At iter-1 time (`c7df8712b`), `execute_node` spanned 3786–6581, about 2,795 lines. It now spans `workflow_engine.rs:3835–6665`, about 2,830 lines. The `review_run` arm grew from 953 to 972 lines (`:5161–6133`). The file grew from 8,589 to 8,725 lines during the iteration-1 fixes (see 2-07). `run_workflow_with_budget` spans `:1850–3257`. |
| 02 `modules.rs` as a service façade | major | Open | The file grew from 7,676 to 7,703 lines. `pub use otto_review::engine::*;` is still at `:39`. `crate::modules::` is still used 42 times from 8 files (workflow_engine ×23, vault_docs_agent ×8, run_engine ×5, …). |
| 03 cancel registry copied 6× | major | **Fixed** | `otto_core::cancel::CancelRegistryExt` and `CancelGuard` (`cancel.rs:25–140`) replace the copies. Locking is poison-tolerant (`lock_map`), and the guard unregisters only when its flag is still the registered one (`ptr_eq`). The remaining per-engine `signal_cancel` functions (`skill_eval.rs:56`, `otto-swarm runtime/run.rs:33`, `otto-product run.rs:429`) are one-line key-scheme wrappers, not copies. |
| 04 "N agents + summarizer" engine re-implemented per feature | major | Open | Each engine still identifies the summarizer its own way: `skill_review.rs:400` checks `agent.name == "summarizer"`, `modules.rs:4621` relies on it being the last row, and `vault_docs_agent.rs:356` uses `SUM_RETRY_IDX = usize::MAX`. `otto-agent-run` gained no fan-out runner. |
| 05 two MCP tool catalogs | major → **minor (residual)** | Partly fixed | `mcp_tools/tests/catalog_tests.rs:16` now checks schema parity for each native/governed pair. `DEFERRED_SCHEMA_DRIFT` may only shrink. Two hand-maintained catalogs remain, plus 7 drifted pairs and the separate native REST dispatch (`format!("/repos/…")` ×5). With the test in place the risk is contained, so the residual is re-rated minor. |
| 06 otto-telemetry → otto-usage | major | Open, with new evidence | `otto-telemetry/Cargo.toml:8` still lists `otto-usage`, and `lib.rs:13` still has `use otto_usage::{ClickHouse, UsageEngine}`. New evidence: `otto-sessions` depends on `otto-git` only for `otto_git::hardened_command()` (`manager.rs:6852`, `resolve_git_common_dir`). So every session-manager build pulls in otto-git → otto-telemetry → otto-usage (the ClickHouse engine). |
| 07 REST engine in a route file; handlers called with fabricated extractors | major | Open, with a second instance | `workflow_api_request.rs:49–66` is unchanged: it builds an `AuthContext` literal and calls `routes::api_client::execute(Path, State, CurrentUser, CurrentAuthContext, headers, Json)`. Second instance: `mcp_capabilities.rs:292–300` calls the `code_search` handler with `AxPath(..)`, `State(ctx.clone())`, `CurrentUser(..)` and `Query(CodeSearchQuery{..})`. `otto-apiclient` still contains only gRPC and scripts. |
| 08 `WorkflowsPage.svelte` | major | Open | 5,145 → 5,158 lines. |
| 09 `GraphView.svelte` | major | Open | 5,320 lines, unchanged. |
| 10 `DatabaseStore` | major | Open | 4,348 → 4,383 lines. |
| 11 shared renderers inside feature modules; markdown rendered 5 ways | minor | Open | `marked` is still imported directly by `WorkflowsPage.svelte`, `panels/FileTree.svelte`, `api/RequestBuilder.svelte` and `kubernetes/monitor/MonitorInsights.svelte`. That is in addition to `lib/md.ts`, `vault/mdRender.ts` and `chatMarkdown.ts`. Other modules import `canvas/mermaid`, `vault/mdRender` or `database/JsonTree` in 21 files. |
| 12 product ↔ design-hall and product ↔ swarm cycles | minor | Open | Six design-hall files import `product/design/*`. `product/types.ts:7` imports `../swarm/types`, and `swarm/types.ts:379` imports `../product/types` inline. |
| 13 `lib/` imports `modules/` | minor | Open | There are still 34 such imports. `lib/api/types.ts:12115` re-exports from `../../modules/product/types`. No guard rule exists (see 2-07). |
| 14 hand-rolled authed `fetch` | minor | **Fixed** | `rg 'Bearer ${getToken()}'` outside `lib/api/client.ts` finds nothing. `authedResponse` and `authedText` replace the copies (FIXES P-UI). |
| 15 review status transitions hand-rolled | minor | Open | `Event::ReviewChanged {` is still built by hand at 17 sites across `modules.rs` and `review_launch.rs`. |
| 16 usage tailer in the binary; Codex model extraction triplicated | minor | Open | `usage_tailer.rs` is 2,424 lines (was 2,388). `"slug"` fallbacks still appear at `usage_tailer.rs:1111`, `otto-transcript/usage.rs:512` and `codex.rs:1074`. |
| 17 `types.ts` single-file hotspot | minor | Open | 12,171 → 12,188 lines. The newer areas already use per-area files (`lib/api/room-types.ts`, `game-room-types.ts`, `room-recap-types.ts`, `db-multirun-types.ts`, `maintenance-types.ts`), so the split pattern exists. It just hasn't been applied to the monolith. |

No iteration-1 architecture finding was rejected.

## Areas covered in this iteration

- Re-verification of all 17 iteration-1 findings, as listed above.
- A 3-week churn and size-trend ranking for `crates/` and `ui/src`, comparing first-commit sizes with
  current ones. The scripts LOC ratchet (`scripts/loc-ratchet.py`, `loc-baseline.json`) and its
  history were read.
- The internal crate dependency graph was re-derived from every `crates/*/Cargo.toml`. The unusual
  edges checked were sessions→git, sessions→ssh and design-assist→improve.
- otto-sessions: the `manager.rs` method-size profile (226 functions, with `create` at 329 lines and
  `restart_locked` at 263), plus the `ws.rs` structure (the `serve_terminal` select loop and
  `ClientFrame` dispatch). Neither produced a finding: the loop is a cohesive protocol handler and
  manager.rs is shrinking.
- otto-dbviewer: the `Driver` trait, the shared `ResourceCache`, the helpers duplicated between the
  mysql and postgres drivers, and SQL classification across `types.rs`, `access.rs` and the drivers.
- otto-server: `policy.rs` (`policy_for` / `route_class`; deliberately central, so no finding), the
  `rooms/` and `game_rooms/` placement and the `ServerCtx` surface they use, the `workflow_engine.rs`
  arm sizes and the `git_pr` arm's use of otto-git, and direct handler calls (`State(ctx.clone())`).
- otto-state `personal_agents.rs`: admission is transactional on purpose, so no finding. otto-rbac
  `tokens.rs`: one cohesive token repo, so no finding. otto-core module sizes and `api/` partial split:
  no finding.
- UI:
  - `lib/components/Terminal.svelte`, including the parked-engine and live socket handlers;
  - `shell/App.svelte`, its effects and palette sources;
  - `lib/stores/workspace.svelte.ts`;
  - `lib/events.svelte.ts`, comparing the dispatch chain with `liveEvents`;
  - `lib/api/mock.ts` and `mockGate`: lazily gated, no finding;
  - the `lib/api/*` layout;
  - the `ui/scripts/ui-guards.mjs` rule set.

## Areas NOT covered yet (next iteration)

- otto-product `service.rs` (3,610) and `run.rs` (3,574), otto-design `store.rs`/`service.rs`, and
  otto-design-assist `lib.rs` (3,290). The dependency from design-assist to otto-improve is noted, not
  analysed.
- otto-issues `jira.rs` (3,706) and `confluence.rs` (2,901): whether they share an HTTP/auth layer.
- otto-dbviewer `drivers/mongodb.rs` (4,975, ratcheted) and `service.rs` (4,104) internals.
- otto-swarm `runtime/engine.rs` (3,500) and otto-usage `engine.rs` (3,291).
- otto-server `ui_bridge.rs` (2,079), `api_helpers.rs` (1,770), `mcp_outward.rs` (2,191) and
  `ws_events.rs`, compared with the UI's `events.svelte.ts` event coverage.
- UI: `product/OverviewTab.svelte` (3,486), `database/ResultsGrid.svelte` (2,580),
  `database/QueryEditor.svelte` (2,220), `git/DiffViewer.svelte` (2,343), `git/WipPanel.svelte`,
  `shell/Navigator.svelte` (2,514), `lib/stores/apiClient.svelte.ts` (1,548 → 2,261 in 3 weeks) and
  `scheduled-tasks/ScheduledTasksPage.svelte` (1,004 → 1,559).
- Whether `docs/contracts/*.md` has drifted from the routes. That belongs to the api-contracts lens,
  but it was listed as uncovered in iteration 1.

---

## New findings

### architecture-2-01: major: `Terminal.svelte` decodes the `/ws/term` protocol twice, inside a 3,632-line component that grew by 28% in 3 weeks

**Location:** `ui/src/lib/components/Terminal.svelte:150-229`, `:943-1147`

**Evidence**

The file grew from 2,829 to 3,632 lines (+803) over 26 commits in 3 weeks. Its instance script runs
from line 232 to 3047 and holds 84 functions:

- the socket protocol: `connect`, `wireSocket`, `scheduleReconnect`, `sendJson`, `resyncOnInput`;
- resize and grid-authority negotiation (`deferLocalResize` … `applyGrid`, `safeFit`; about 450
  lines);
- WebGL lifecycle, find, links, touch, paste upload and engine park/adopt.

The terminal frame protocol is decoded by hand in two places.

1. The module-level `wireParked` (`:150-229`) handles `credit`, `scrollback` (binary header and
   base64), `status`, `exit`, `error` and `grid`. Its frame type is an inline anonymous type
   (`:188`).
2. The instance `wireSocket` `onmessage` (`:986-1070`) handles the same frames, plus `probe_ack` and
   `search_result`. It uses an untyped `JSON.parse`.

The two copies already differ. The live `status` case clears `dormantView` and sets the
reconnectable or exited overlay (`:1024-1046`). The parked one only stores the status (`:208-212`).
The binary-snapshot handshake (a header frame, then the next binary frame as payload) is coded twice:
`binarySnapHeaders.get/set` at `:177-182`/`:199-201` and again at `:993-998`/`:1016-1021`.

Each copy also builds its own snapshot-apply path (`applyParkedSnapshot` `:160` and
`applySnapshot` `:897`). `lib/api/types.ts` already defines the typed frames (`WsTermGridFrame`,
`WsTermProbeAckFrame`, …), but neither handler parses into a discriminated union.

**Cost**

Every session view runs through this component.

- A change to a terminal frame or to snapshot handling has to be made in both decoders. The parked
  path is only exercised when a pane is detached and later re-adopted, so a missed copy shows up as
  "a parked terminal comes back garbled".
- None of the protocol logic can be unit-tested without mounting xterm.

**Fix**

1. Add `lib/components/termProtocol.ts`. It should contain:
   - `decodeTermFrame(ev: MessageEvent, sock): TermFrame | {kind:'bytes'} | {kind:'snapshot', epoch, bytes}`,
     owning the binary-header state and returning a discriminated union typed from `lib/api/types`;
   - one `applySnapshot(target: {term, writes, flow}, epoch, bytes)` used by both the live and the
     parked engine.
2. Make `wireParked` and `wireSocket` thin `switch`es over the union.
3. Next, move the resize/claim negotiation (`deferLocalResize` … `safeFit`) into a `termGridSync.ts`
   controller, following the `termFlow` / `termPark` / `termCompactQueue` pattern already in this
   folder.

### architecture-2-02: minor: SQL statement classification is copied four times in otto-dbviewer, with different keyword sets

**Location:** `crates/otto-dbviewer/src/types.rs:1528-1545`, `drivers/mysql.rs:2022-2050`, `drivers/postgres.rs:1709-1735`, `drivers/clickhouse.rs:2417-2432`

**Evidence**

The comment-skipping "first keyword" helper exists three times:

- mysql `first_keyword` (`:2032`);
- postgres `first_keyword` (`:1717`);
- `types.rs` `strip_leading_comments` (`:1293`) and `sql_first_keyword`. Its doc comment says
  "(Local copy of the driver helper to keep the gate engine-agnostic)".

The "is this a read?" rule exists four times with four different lists:

| Location | Keywords treated as reads |
|----------|---------------------------|
| `types.rs:1538` `sql_first_keyword_is_read` | `SELECT SHOW DESC DESCRIBE EXPLAIN WITH USE` |
| mysql `is_read_statement` | `SELECT SHOW DESC DESCRIBE EXPLAIN`, plus `WITH` only when `access::read_is_provable` |
| postgres `is_read_statement` | `SELECT WITH SHOW EXPLAIN TABLE VALUES`, with `WITH` unconditional |
| clickhouse `returns_rows` | a tokenizer-based check that adds `EXISTS CHECK KILL BACKUP RESTORE` |

So the same `WITH x AS (DELETE …) SELECT …` is classified differently by:

- the postgres run path (`postgres.rs:691`, the read path);
- the export gate (`postgres.rs:855`);
- the guardrail (`types.rs:1459-1463`, which treats it as read only when provable).

The two drivers also carry byte-identical copies of `float_to_json` (`mysql.rs:2798`,
`postgres.rs:2440`) and `acquire_error`. `mysql.rs:600` reaches into
`crate::drivers::postgres::{assemble_bulk_graph, BulkCol, BulkFk, BulkRel}`.

**Cost**

A fix to classification has to be applied up to four times. Examples: MySQL `#` comments, nested
Postgres block comments, a newly recognised read keyword, or a data-modifying CTE. The guardrail and
the execution path can then disagree about whether a statement is a read.

**Fix**

Add `otto-dbviewer/src/sql_class.rs` with:

- `first_keyword(sql) -> String`, the one comment-aware scanner;
- `ReadClass::of(engine, sql) -> {Read, ProvableRead, Write, Unknown}`, built on `access::read_is_provable`
  for MySQL and Postgres and on the tokenizer for ClickHouse.

Drivers and `types.rs` should call it. The per-engine keyword lists become a table in that file. Move
`float_to_json`, `acquire_error` and the bulk-graph assembler into `drivers/sql_common.rs`.

### architecture-2-03: minor: the shell's workspace store has absorbed feature settings and workflow-run tracking, parsing untyped settings with casts

**Location:** `ui/src/lib/stores/workspace.svelte.ts:2171-2224`, `:471-522`

**Evidence**

The store grew from 1,483 to 2,227 lines (+50%) over 29 commits in 3 weeks. Besides workspaces,
sessions, tabs and splits, it now owns:

- **API-client settings.** It has getters and setters for `apiAllowLocal`, `apiHistoryRetention` and
  `apiRunsKeep` (`:2171-2216`). Each reads `this.current?.settings?.api_client as { allow_local?:
  boolean }` and similar casts, because `Workspace.settings` is `Record<string, unknown>`
  (`lib/api/types.ts:26`).
- **Workflow-run tracking.** It owns `activeWorkflowRuns`, `refreshActiveWorkflowRuns` and
  `applyWorkflowRunEvent` (`:471-522`), and it imports `listActiveWorkflowRuns` from
  `lib/api/workflows`.
- **The default-agent setting** (`saveDefaultAgent`).

Its consumers are feature modules: `modules/api/{RetentionDialog,HistoryList,RequestBuilder,ResponseViewer}`
(12 uses) and `modules/workflows/WorkflowsPage.svelte` (7).

**Cost**

Every feature with a per-workspace setting adds untyped accessors to the shell's core store. A
renamed settings key fails silently, because the cast hides it from `npm run check`. The core store's
change rate is driven by unrelated features.

**Fix**

1. Type the settings: add `WorkspaceSettings { api_client?: ApiClientWsSettings; default_provider?: string; notes?: string; … }`
   to `lib/api/types` (and the Rust contract).
2. Keep only `patchSettings(wsId, partial)` and `current.settings` on `ws`.
3. Move the API-client accessors into `lib/stores/apiClient.svelte.ts` and the active-runs list into
   a `workflowRuns` store that `events.svelte.ts` updates.

### architecture-2-04: minor: WebSocket events are routed by a 230-line hand-written `if/else` chain beside an existing subscription registry

**Location:** `ui/src/lib/events.svelte.ts:706-965`

**Evidence**

`onmessage` passes each event first to the generic registry (`liveEvents.dispatch(parsed)`, `:730`;
`liveEvents = appLive`, `:401`, with subscribers registering through `liveEvents.on(types, fn)`). It
then runs a second, hard-coded chain of `parsed.type === '…'` branches (67 type comparisons, from
`:731` to about `:960`) into 14 statically and lazily imported feature stores (`:16-50`). The file
grew from 830 to 997 lines over 20 commits in 3 weeks.

**Cost**

There are two dispatch mechanisms. Each new event type means editing this central file, and an event
type the chain doesn't list is silently ignored. There is no exhaustiveness check against the
`OttoEvent` union.

**Fix**

Replace the chain with a typed route table:

```ts
const routes: { [K in OttoEvent['type']]?: (ev: Extract<OttoEvent, {type: K}>) => void }
```

Split it per feature, for example `lib/events/routes/{swarm,product,database,…}.ts`, each keeping its
own `lazyModule` loader. Add a unit test asserting that every `OttoEvent['type']` either has a route
or is listed as handled through `liveEvents` only.

### architecture-2-05: minor: the shell root owns feature palette commands and 49 side-effect `$effect`s

**Location:** `ui/src/shell/App.svelte:669-940`

**Evidence**

`App.svelte` has 49 `$effect(` blocks and 14 `registry.register(` palette sources. The file grew from
1,523 to 1,716 lines over 31 commits. The palette sources include feature data that other modules
register for themselves:

- `repos` (`git.repos` → `router.go('git/…')`);
- `connections`, which fetches `/workspaces/{id}/connections` and POSTs `/connections/{id}/open`
  (`:904-940`);
- workspaces, sessions, the focused session, settings sections and help guides.

Twenty other modules already register their own palette sources (for example `git/GitPage.svelte`,
`vault/VaultPage.svelte` and `mcp/McpPage.svelte`).

This shape has already cost something: a root boot `$effect` that read `router.module` re-ran
`auth.boot()` and remounted the whole shell on every sidebar switch (fixed in `c14b1d5e`).

**Cost**

New global commands and installers keep landing in the app root. That is where one stray reactive
read re-runs boot code, and where ordering between 49 effects is implicit.

**Fix**

1. Move the palette sources to `shell/paletteSources.svelte.ts` as `installPaletteSources(): () => void`.
2. Move the connections and repo commands into the connections and git modules' command files.
3. Collect the one-shot global installers (menu bridge, nav prefetch, key map, badge) into a single
   `installShell()` called from one `onMount`.

App.svelte then keeps only layout and the reactive state it renders.

### architecture-2-06: minor: rooms and game rooms (about 9.7k lines) were built inside otto-server, although they use only six ctx fields

**Location:** `crates/otto-server/src/rooms/` (8,174 lines), `crates/otto-server/src/game_rooms/` (1,505 lines)

**Evidence**

Rooms include the room registry, ICE and presence, the terminal relay, and the recap pipeline. The
recap pipeline has archive, media and consent parts, plus `recap_engines/{speech,vision,summary,process}`.
Rooms were added on 2026-09-27 and game rooms on 2026-10-08, both inside otto-server.

Their whole dependency on the server is:

- `ctx.rooms`, `ctx.game_rooms`;
- `ctx.pool`, `ctx.manager`, `ctx.secrets`, `ctx.base_url`, `ctx.data_dir`, `ctx.audit`;
- `crate::ServerCtx` for the axum state.

They import no other otto-server module (`rg 'crate::'` finds only `crate::rooms`). The contract
types already live in otto-core (`api/rooms.rs`, `api/recap.rs`).

Meanwhile otto-server is under a total-LOC ratchet ("may only shrink", `scripts/loc-baseline.json`
`totals.otto-server`). That ratchet had to be raised for a rooms change: `22ea1356f` "allow the 2
rustfmt lines of the new kart track ids in the otto-server LOC ratchet". The crate table in AGENTS.md
shows the established direction: domain engines live in their own crate behind a `*Ctx` trait
(otto-review, otto-automation, otto-swarm, otto-insights, …).

**Cost**

Every rooms or games iteration recompiles and re-tests otto-server, the slowest unit in the
workspace. The games team is iterating now: new modes and maps were requested. The ratchet will keep
needing manual raises.

**Fix**

1. Create `crates/otto-rooms`, containing `rooms/`, `game_rooms/` and the recap engines, behind a
   `RoomsCtx` trait (`pool()`, `sessions()`, `secrets()`, `base_url()`, `data_dir()`, `audit()`).
2. Write the `impl RoomsCtx for ServerCtx` in otto-server, next to the other ctx impls.
3. Keep only the route mounting in otto-server: `rooms::protected_routes()`, `public_routes()` and
   `ws_routes()`.

### architecture-2-07: minor: the god-module guard counts the wrong lines, and the UI has no guard at all

**Location:** `scripts/loc-ratchet.py:1-60`, `scripts/loc-baseline.json`, `ui/scripts/ui-guards.mjs`

**Evidence**

The Rust ratchet caps whole-file line counts, tests included. Moving `#[cfg(test)]` blocks out of a
tracked file therefore creates headroom that production code can then use:

- **workflow_engine.rs:**
  - Before iteration 1, the baseline was 10,122 lines while the file was 8,589 (`c7df8712b`).
  - The iteration-1 fixes grew the file to 8,725 lines (+136). `execute_node` went from 2,795 to
    2,830 lines and the `review_run` arm from 953 to 972.
  - All of that passed under the old cap. `--update-baseline` then lowered the cap to the new, larger
    count (`66b55a25c`).
- **modules.rs:** grew from 7,676 to 7,703 lines under a cap of 7,749.
- **otto-server total:** the cap was raised from 130,939 to 131,580 lines.

On the UI side, `ui-guards.mjs` ratchets only style rules: tokens, pills, spinners, intervals and so
on. It has no file-size rule and no `lib/` → `modules/` layering rule. The UI god files grew
unchecked in 3 weeks:

| File | Lines (3 weeks ago → now) |
|------|---------------------------|
| `Terminal.svelte` | 2,829 → 3,632 |
| `workspace.svelte.ts` | 1,483 → 2,227 |
| `apiClient.svelte.ts` | 1,548 → 2,261 |
| `ScheduledTasksPage.svelte` | 1,004 → 1,559 |
| `WorkflowsPage.svelte` | 4,625 → 5,158 |

**Cost**

The one mechanism meant to stop findings 01, 02 and 08–10 from getting worse can be sidestepped on the
Rust side and is missing on the UI side. Every iteration therefore adds to the deferred splits.

**Fix**

1. In `loc-ratchet.py`, count production lines only: stop at the first top-level `#[cfg(test)]`, or
   exclude `mod tests` blocks.
2. Add a per-function cap for the tracked god functions (`execute_node`, `run_workflow_with_budget`,
   `run_review_core`).
3. Give `ui-guards.mjs` a `file-loc` ratchet over `src/**/*.{svelte,ts}` with a `new_file_max`.
4. Give it a `lib-imports-modules` rule, baselined at today's 34 imports (closes the guard half of
   finding 13).

### architecture-2-08: minor: game map and capacity ids are hard-coded in otto-server and mirrored by hand in the UI and the docs

**Location:** `crates/otto-server/src/game_rooms/types.rs:10-25`, `crates/otto-server/src/game_rooms/registry.rs:138`

**Evidence**

`GameKind::maps()` hard-codes the map ids:

- `Shooter => &["isle", "mesa"]`;
- `Kart => &["meadow", "downtown", "canyon", "frost", "lagoon", "volcano", "skyway"]`.

`capacity()` hard-codes 4 and 8. `registry.rs:138` returns 400 for any other map. The same ids are
maintained separately in three more places:

- `docs/contracts/game-rooms.md:18-19`;
- `ui/src/modules/rooms/games/shooter/catalog.ts:2-5`, which declares `ShooterMapId = 'isle' | 'mesa'`;
- `shooter/net.ts:112`, which checks `s.m !== 'isle' && s.m !== 'mesa'`.

Adding three kart tracks needed an otto-server edit and an otto-server LOC-ratchet raise (`22ea1356f`).

**Cost**

The user has just asked for more shooter modes and maps, including call-of-duty and apocalypse-style
content. Each new map then means editing four hand-synced lists, including a rebuild of the heaviest
crate. If the lists drift, the server answers 400 for a map the UI offers.

**Fix**

1. Keep one `game-catalog.json` (kinds, map ids, capacities), for example in `docs/contracts/` or
   `crates/otto-core/assets/`.
2. On the server, load it with `include_str!` and serde in `GameKind::maps()` / `capacity()`.
3. On the UI side, import the JSON. The ids then come from one source, and `catalog.ts` adds only the
   presentation fields (names, blurbs, covers).
4. A unit test on each side pins that the catalog is non-empty and matches the contract table.
5. This is cheapest to do as part of 2-06.

---

## Carried-over findings (still open; full text in `../iter-1/architecture.md`)

The refactor direction for each is unchanged from iteration 1 unless noted.

| ID | Severity | Location | One-line summary and any new direction |
|----|----------|----------|----------------------------------------|
| architecture-1-01 | major | `crates/otto-server/src/workflow_engine.rs:3835` | `execute_node` has grown to about 2,830 lines. Split it into `workflow_engine/nodes/<kind>.rs`, starting with `review_run` (972 lines), `git_pr` (366) and `loop` (308). |
| architecture-1-02 | major | `crates/otto-server/src/modules.rs:39` | A 7.7k-line review, drafting, session-input and browser-proxy file behind a glob re-export. |
| architecture-1-04 | major | `crates/otto-server/src/skill_review.rs:400`, `vault_docs_agent.rs:356`, `modules.rs:4621` | Fan-out plus summarizer is re-implemented per feature, with three different ways to identify the summarizer. |
| architecture-1-06 | major | `crates/otto-telemetry/Cargo.toml:8`, `crates/otto-sessions/src/manager.rs:6852` | The telemetry API depends on the ClickHouse usage engine. New direction: also move `hardened_command()` out of otto-git into a leaf crate so otto-sessions can drop its otto-git dependency. |
| architecture-1-07 | major | `crates/otto-server/src/workflow_api_request.rs:59`, `crates/otto-server/src/mcp_capabilities.rs:292` | Internal callers invoke axum handlers with fabricated extractors. There are now two such sites. |
| architecture-1-08 | major | `ui/src/modules/workflows/WorkflowsPage.svelte:1` | A 5,158-line god component. |
| architecture-1-09 | major | `ui/src/modules/git/GraphView.svelte:1` | A 5,320-line component mixing lanes, ref sidebar and every git action. |
| architecture-1-10 | major | `ui/src/lib/stores/database.svelte.ts:1` | A 4,383-line singleton store. |
| architecture-1-05 | minor (residual) | `crates/ottod/src/mcp_tools.rs:1665`, `crates/ottod/src/mcp_tools/tests/catalog_tests.rs:16` | Two hand-maintained MCP catalogs. A parity test now contains the drift, and 7 pairs are deferred. |
| architecture-1-11 | minor | `ui/src/modules/panels/FileTree.svelte`, `WorkflowsPage.svelte`, `api/RequestBuilder.svelte`, `kubernetes/monitor/MonitorInsights.svelte` | Direct `marked` use, with shared renderers owned by feature modules. |
| architecture-1-12 | minor | `ui/src/modules/product/types.ts:7`, `ui/src/modules/swarm/types.ts:379` | Product ↔ swarm and product ↔ design-hall import cycles. |
| architecture-1-13 | minor | `ui/src/lib/api/types.ts:12115` | 34 imports from `lib/` into `modules/`. |
| architecture-1-15 | minor | `crates/otto-server/src/modules.rs` (17 `Event::ReviewChanged` sites) | Review status changes are persisted and broadcast by hand at every exit path. |
| architecture-1-16 | minor | `crates/ottod/src/usage_tailer.rs:1111` | The tailer driver lives in the binary, and Codex model extraction exists three times. |
| architecture-1-17 | minor | `ui/src/lib/api/types.ts:1` | A 12,188-line single-file contract mirror. |
