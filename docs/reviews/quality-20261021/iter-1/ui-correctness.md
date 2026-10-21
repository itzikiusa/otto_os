# UI correctness review: iteration 1

Lens: logic bugs in the Svelte 5 UI (`ui/src/**`). This covers runes and reactivity, races between async loads and navigation, state left broken after errors, and API usage that does not match `ui/src/lib/api/types.ts`.
Branch `fix/quality-loop-20261021` @ `d447516c4`. The review was read-only: no builds or tests were run. `ui/src/modules/rooms/games/**` was excluded.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 5 | 1.50 |
| minor | 3 | 0.30 |
| nit | 0 | 0.00 |

**Score = 10.0 - 1.5 - 0.3 = 8.2 / 10**

## Previous-finding status

This is iteration 1, so there are no earlier findings to track.

## Areas covered

- **Recent churn:** I checked the last 3 weeks of UI commits, mainly 452ade172, 18a21bfd0, f99242740, eb295dcc2 and c5673da67.
  - Workspace `settings_patch` migration. I checked the daemon's RFC 7396 `json_patch`; it is correct.
  - Activity store snapshot and delta merge.
  - Browser summary fields.
  - Handover, TiledView and SplitNode RTL changes.
  - Git landed marks (`merged_into`).
  - The `sql-util` MySQL `DELIMITER` handling and `looksLikeMongoshScript`.
- **API paths:** I matched every `api.get/post/put/patch/del` path in `ui/src` against all 1117 routes declared in `crates/**`. No path was missing; the 49 leftover mismatches were all `${qs}` query-suffix false positives.
- **Workspace-scoped loaders:** I compared the stores for a latest-wins / workspace guard: scheduledTasks, loops, runWithOtto, personalAgents, swarm, proof, git, browser, database (connections, saved queries, history, dashboards) and brokers.
- **Keyed `{#each}` blocks:** I swept for keys that can be duplicated by server data or user input. Svelte 5.57.1 throws `each_key_duplicate` in production builds too (`errors.js:146`), and the UI has no `<svelte:boundary>` anywhere.
- **Busy flags:** I used a heuristic scan for flags set before an `await` with no `finally`, then confirmed each hit by hand.
- **Components read:**
  - Shell `App.svelte` effects (route→store sync, needs-you flag, prefetch).
  - SessionView effects and key handlers.
  - The WipPanel / commitComposer draft lifecycle. I dropped a suspected bug here: the panel is remounted per repo via `{#key activeRepo.id}`.
  - RoomPage, RoomHost, RoomGuest, RoomMedia and RoomChat.
  - SqsView.
  - WorkflowsPage run, approval and progress effects.
  - InsightsPage load and poll.
  - RefineChat turn ownership.
  - ActivityPanel, FileTree, ScheduledTasksPage, AttributionDrilldown, Dashboards, MultiRunDialog and BrokersPage.

## Areas not yet covered (next iteration)

- `lib/components/Terminal.svelte` (2.6k lines) and `lib/events.svelte.ts` WS reconnect and resync ordering. I only read these in part.
- `ResultsGrid` / `GridView` edit flow (`EditFlow.svelte.ts`) and the `QueryEditor` run and cancel path in `database.svelte.ts` (`runQuery` at line 3039 and onward).
- Kubernetes (`ClusterWorkspace`, `monitor/*`), AWS views other than SQS, and `mission-control`, `canvas`, `design-hall`, `vault` (`VaultPage`, `DocsAgentsView`), `personal-agents` and `share`.
- Navigator session-row actions and the `TabBar` drag and reorder.
- Other keyed `{#each}` over server lists whose uniqueness I did not prove:
  - `PrMergeModal` `checks.checks (c.name)`: GitHub check-run names are not unique across workflows.
  - `MetricChart` `series (s.label)`.
  - `ConnectionImportDialog` `warnings (w)`: two skipped profiles with the same name produce the same warning text.

## Findings

### ui-correctness-1-01: major: Multi-run result table crashes on duplicate column names
- **File:** `ui/src/modules/database/MultiRunDialog.svelte:707`
- **Evidence:** `<tr>{#each detail.result.columns as col (col.name)}<th>{col.name}</th>{/each}</tr>`. SQL result column names are not unique: `SELECT a.id, b.id …`, `SELECT 1, 1`, or `SELECT *` over a join. The daemon passes them through as-is. In Svelte 5 a duplicate key throws `each_key_duplicate` in production too (`node_modules/svelte/src/internal/client/dom/blocks/each.js:356-361` → `errors.js:146`), and the app has no `<svelte:boundary>`. The main grid already avoids this: `GridView.svelte:820` and `VerticalView.svelte:462` key by index.
- **Failure:** Run a multi-run whose statement joins two tables that share a column name, then expand a run's result. The render effect throws and the detail never renders. The error escapes to the global handler.
- **Fix:** Key by index: `{#each detail.result.columns as col, ci (ci)}`.

### ui-correctness-1-02: major: "Add widget" sheet crashes when the active result has duplicate column names
- **File:** `ui/src/modules/database/Dashboards.svelte:245` and `:254`. The column list comes from `:92` (`resultCols = database.tab.result?.columns`).
- **Evidence:** `{#each resultCols as c (c.name)}<option …>` appears twice: the X/category select and the Y/value select. Both list the active tab's result columns. `openAdd()` (`:60`) seeds the sheet from that tab ("save this result as a widget"), so a join result with a repeated column name opens a sheet that throws `each_key_duplicate`.
- **Failure:** Run `SELECT * FROM orders o JOIN users u ON …` (both tables have `id`), then Dashboards → Add widget. The sheet fails to render.
- **Fix:** Key by index (`as c, i (i)`), or dedupe names for the option list (`[...new Set(resultCols.map(c => c.name))]`). A widget mapping is by name, so the deduped list is the correct option set.

### ui-correctness-1-03: major: Duplicate golden-task tags permanently break the Golden tasks list
- **File:** `ui/src/modules/skills-eval/GoldenTasksView.svelte:413`. The tags are written at `:185` and `:194`.
- **Evidence:** Save builds `rawTags = fTags.split(',').map(s => s.trim()).filter(Boolean)` with no dedupe. The daemon stores the array verbatim: `crates/otto-server/src/eval_lab_routes.rs:35` passes `tags: req.tags.clone()`, and `otto-state/src/eval_lab.rs:83` writes `serde_json::to_string(&input.tags)`. The list then renders `{#each t.tags as tag (tag)}`.
- **Failure:** Type `auth, auth` (or `go,go`) in Tags and save. Every later render of the list throws `each_key_duplicate`. You cannot fix the task through the UI, because the list that holds its Edit button no longer renders.
- **Fix:** Dedupe on save (`[...new Set(rawTags)]`) and key the chips by index (`as tag, i (i)`) so existing rows already saved with duplicates still render. Optionally, the daemon can also dedupe in `eval_lab_routes.rs`.

### ui-correctness-1-04: major: DB dashboards and widgets loads have no workspace or latest-wins guard
- **File:** `ui/src/lib/stores/database.svelte.ts:4145-4182` (`loadDashboards` / `fetchDashboards` / `loadWidgets`).
- **Evidence:**
  - `fetchDashboards` captures `base` but assigns `this.dashboards = await api.get(...)` and then `this.widgets = await api.get(...)` (`:4160`, `:4177`). It does not check `base === this.wsBase()` or `accessEpoch` afterwards.
  - The in-flight dedupe (`dashboardsInFlight.key`) only coalesces requests for the same workspace. A switch starts a second, parallel fetch, and whichever request resolves last wins.
  - A workspace switch never clears `dashboards`/`widgets`. They are reset only on an identity change (`:1241`).
  - The sibling `loadSavedQueries` (`:3942`) has exactly this guard (`savedQueriesScope`, a request counter, `base === this.wsBase()`), as do scheduledTasks, loops, runWithOtto, swarm and proof.
  - The server scopes dashboards per workspace (`otto-state/src/db_explorer.rs:365`, `WHERE workspace_id = ?`).
- **Failure:**
  - **Late response:** Open Connections in workspace A, then switch to B before A's dashboards response arrives. A's dashboards and widgets land under B.
  - **Failed load:** If B's load fails, A's list stays on screen under "Couldn't load".
  - **Wrong target:** Rename and Delete in `Dashboards.svelte` use `/db/dashboards/{id}`, which is not workspace-scoped. A user in B can therefore rename or delete A's dashboard. Add widget posts B's `base` with A's `dashboard_id`.
  - **Spinner:** A's `finally` also clears `dashboardsLoading` while B is still loading.
- **Fix:**
  - Mirror `loadSavedQueries`. Keep a `dashboardsScope` and clear `dashboards`, `widgets`, `selectedDashboardId` and `dashboardsError` when `base` changes.
  - Take a request ticket.
  - Commit `dashboards`, `widgets`, `selectedDashboardId`, `dashboardsError` and `dashboardsLoading` only while `ticket === current && base === this.wsBase() && accessEpoch === this.accessEpoch`. Apply the same check to `loadWidgets`.

### ui-correctness-1-05: major: `brokers.load` lets another workspace's clusters land after a switch
- **File:** `ui/src/lib/stores/brokers.svelte.ts:31-56`
- **Evidence:** `this.clusters = await api.get(\`/workspaces/${wsId}/brokers/clusters\`)` (`:34`). The load has no sequence number and no `wsId === ws.currentId` check, and the same applies to `this.sections` (`:50`). There are two callers:
  - `BrokersPage.svelte:64-67`, an effect on `ws.currentId`.
  - The `DatabasePage.svelte:742-757` effect, which also calls it per workspace.

  On error the comment says "the previous list stays", and that list belongs to the previous workspace.
- **Failure:**
  - **Late response:** Switch A → B while A's request is in flight. A's clusters replace B's, and `openIds` / `selectedId` are pruned against the wrong list.
  - **Failed load:** If B fails, A's clusters stay listed in B with an error banner.
  - **Wrong target:** Test, Remove (`/brokers/clusters/{id}`) and Move act on A's cluster from B's page.
- **Fix:** Use the store pattern used elsewhere. Track `loadedWs`, clear `clusters`/`sections`/`openIds`/`selectedId` when the workspace changes, and take a `latestOnly()` ticket. Commit the list, error, loading and sections only while the ticket is current and `wsId === ws.currentId`.

### ui-correctness-1-06: minor: Editing a widget closes the sheet and drops the edit when the save fails or is rejected
- **File:** `ui/src/modules/database/Dashboards.svelte:103-115`, with `ui/src/lib/stores/database.svelte.ts:4266-4274`
- **Evidence:** `database.updateWidget` returns `void` in every case. It returns early after a toast when `widgetStatementOk` rejects a write verb, and it swallows HTTP errors into a toast. `saveWidget` then always runs `saving = false; adding = false; editing = null;`. The create path handles this correctly with `if (w) adding = false` (`:125`).
- **Failure:** Edit a widget's statement into `UPDATE …` (rejected as "Widgets must be read-only"), or save while the daemon errors. A toast appears, the sheet closes, and the edited title, statement and mapping are lost.
- **Fix:** Have `updateWidget` return `DbWidget | null`, the same as `createWidget`. Close the sheet only on success, and keep `saving` in a `try/finally`.

### ui-correctness-1-07: minor: ⌘⇧C does nothing when the active session is shown in two split panes
- **File:** `ui/src/modules/agents/SessionView.svelte:327-338`
- **Evidence:** Every mounted SessionView adds a `window` keydown listener, guarded only by `ws.activeSessionId !== sessionId`. `splitLayout.splitFocused` (`splitLayout.svelte.ts:215-222`) clones the focused session into the new pane, so a split starts with the same session in both panes, which is the default. `Splits.svelte:109` notes this same duplication. Both listeners pass the guard and both call `toggleView()` → `transcript.setView(sessionId, other(view))` on the shared `$state` map (`transcript.svelte.ts:586`, `:707`). The second call reads the already-flipped view and flips it back.
- **Failure:** Press ⌘D to split, then ⌘⇧C. The view stays Terminal (or Chat), so the documented shortcut (keyboard-shortcuts.md:79) silently does nothing.
- **Fix:**
  - Add `if (e.defaultPrevented) return;` at the top of `onKey`; the first handler already calls `preventDefault()`.
  - Alternatively, react only in the pane that owns focus (`focused || (no pane focused && first pane)`).

### ui-correctness-1-08: minor: A note with repeated front-matter tags crashes the structured note view
- **File:** `ui/src/modules/vault/StructuredNote.svelte:53`. The tags come from `crates/otto-vault/src/parse.rs:56-74` and `:96`.
- **Evidence:** `{#each model.tags as t (t)}`. The model's tags are `note.meta.tags` (`structuredNote.ts:303`). The daemon's `str_list` trims and strips `#` but does not dedupe front-matter tags, so `tags: [api, "#api"]` or `tags: "a, a"` produce `["api","api"]`. Inline `#tags` are deduped (`parse.rs:265`); front-matter tags are not.
- **Failure:** Open an OKF note whose front matter repeats a tag (common in agent-written vault docs). The structured view throws `each_key_duplicate`.
- **Fix:** Key by index (`as t, i (i)`) in the UI. Ideally also dedupe in `str_list`, preserving order, so tag counts and the graph agree.

## Rejected candidates (checked and dropped)

- **WipPanel `drafting` across a repo switch:** not a bug. `RepoView` is under `{#key activeRepo.id}` (GitPage:380 / GitPanel:55), so the panel remounts per repo.
- **SqsView region and account state:** not a bug. The view is under `{#key \`${account.id}/${routeService}/…\`}`.
- **`resourceAccess.load` inside DatabasePage `$effect` (`:228`):** no loop. The entry check reads `untrack`'d data, and `key()` only reads identity.
- **InsightsPage mount effect setting `disposed` on rerun:** no rerun happens. The effect has no synchronous reactive reads.
- **Workspace `settings_patch` nested merge:** correct. The daemon uses SQLite `json_patch` (RFC 7396), so `api_client.allow_local` no longer clobbers its sibling keys.
- **API path ↔ route parity:** every UI path matches a daemon route.
