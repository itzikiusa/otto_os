# UI correctness review: iteration 2

Lens: logic bugs in the Svelte 5 UI (`ui/src/**`). This covers runes and reactivity, races between async loads and navigation, state left broken after errors, and API usage that does not match `ui/src/lib/api/types.ts`.
Branch `fix/quality-loop-20261021` @ `ab0a86f05`. The review was read-only: no builds or tests were run. Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`, `ui/src/modules/home/boxes/ClassroomsBox.svelte`.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 4 | 1.20 |
| minor | 4 | 0.40 |
| nit | 1 | 0.03 |

**Score = 10.0 - 1.20 - 0.40 - 0.03 = 8.37 / 10**

## Previous-finding status (iteration 1)

All 8 findings from iteration 1 are fixed in the current code.

| ID | Status | Evidence |
|---|---|---|
| ui-correctness-1-01 | Fixed | `MultiRunDialog.svelte:707` now keys by index: `{#each detail.result.columns as col, ci (ci)}` |
| ui-correctness-1-02 | Fixed | `Dashboards.svelte:96`: `resultColNames = [...new Set(resultCols.map(c => c.name))]` feeds the option lists |
| ui-correctness-1-03 | Fixed | `GoldenTasksView.svelte:186` de-duplicates tags on save; `:414` keys chips by index |
| ui-correctness-1-04 | Fixed | `database.svelte.ts:4147-4211`: `dashboardsScope`, `dashboardsRequest` and `widgetsRequest` tickets, and `current()` checks workspace, ticket and access epoch |
| ui-correctness-1-05 | Fixed | `brokers.svelte.ts:36-80`: `loadedWs` clears lists on a workspace change, plus `latestOnly()` and `wsId === ws.currentId` |
| ui-correctness-1-06 | Fixed | `updateWidget` returns `DbWidget \| null` (`:4299`); `Dashboards.svelte` closes the sheet only on success, inside `try/finally` |
| ui-correctness-1-07 | Fixed | `SessionView.svelte:334`: `if (e.defaultPrevented) return;` |
| ui-correctness-1-08 | Fixed | `StructuredNote.svelte:53` keys by index |

Counts: 8 fixed, 0 still open, 0 rejected as invalid.

## Areas covered

- **Iteration 1 fixes (`66b55a25c`), re-read for regressions:**
  - The `auth.svelte.ts` capabilities retry and backoff, and the App caps banner wired to `appLive.onConnection`.
  - `client.ts` `problemOf` / `authedResponse`.
  - QueryEditor tab/close button split and Delete-to-close.
  - apiClient `importParsed` / `gitPullCollections` result accounting.
  - uiControl `WriteMemory`.
  - ProofPage `packRepoPath`.
  - RoomPage `hostRequests` announcements.
  - QuickOpen combobox.
  - Workbench `closeSidePanel`.
  - Product PlanTab and AnalysisTab poll health.
  - JiraIssuePicker / AttachIssue provider, url and status.
  - NewSession browser providers.
  - ConflictResolverView load error.
  - FindingActions, InfoPanel, McpServers rollback, BarHost lazy bar, LinksPanel, ReportDetail bridge.
- **Events socket (`lib/events.svelte.ts`):** connect, reconnect and backoff, `onAuthChanged` while connecting, detach on stop and reconnect, lag `resync` debounce, `resyncStores`, and the `hello_ack` boot-id restart path. No defect found.
- **DB Explorer run path:**
  - `runQuery` (abort and supersede via `runControllers`, write-confirm paths, the `stopped()` predicate, re-attach on a lost wait).
  - `abortQuery` / `abortRunForTab` / `sendCancel`.
  - `runPage`.
  - `EditFlow.resolveTarget` generation guard.
  - `loadChildren` / `openObject` staleness, and the snapshot capture that drops in-flight loading flags.
- **Dashboards end to end:** `loadDashboards`, `fetchDashboards`, `loadWidgets`, `WidgetCard` run, auto-refresh and abort, `widgetGate` (`createLimiter`), and the Home `DbDashboardBox`.
- **Kubernetes:** the k8s store (`loadClusters`, `loadNamespaces`, `loadResources` abort/304 path, `suspend`, `onAccessChange`), ClusterWorkspace load and auto-refresh effects, and the KubernetesPage access-revision `{#key}`.
- **Home boxes:** `poll.ts` `freshness`, `K8sBox`, `DbDashboardBox`.
- **Pages and stores:**
  - Mission Control (page reload ticket, workspace-switch reset, live debounce, backfill; WorkItemDetail ownership).
  - Share page (generation / `isCurrent`, OTP, extend, access re-check).
  - Vault store (`load`, `select`, `refreshStatus`, `refreshOpenNote`, `loadRoot`, `openNoteInPlace`, `loadFile`, backlinks, search sequence).
  - Personal-agents store (agents, schedules, runs, rooms, messages paging).
  - Canvas store (`loadScenes`, `open`, `refreshSession`).
  - Transcript store (`readTail`, `loadEarlier`, epoch reset).
  - Swarm settings (standing goals, triggers).
  - Skills-eval MatrixView and list.
- **Shell:** TabBar drag and keyboard reorder and rename, Navigator bulk archive/delete and archived paging (`ws.loadArchived` generation), and `git.loadRepos` / `loadAllRepos`.
- **Keyed `{#each}` sweep:** I listed 387 keyed blocks with string keys and traced the risky server-fed ones to their producers:
  - **Duplicates confirmed (findings below):** `PrMergeModal` checks, `ConnectionImportDialog` warnings, `MultiRunDialog` plan warnings.
  - **Checked and not reported:** `PrDetail` reviewers (deduped in `github.rs:739`), AWS `LogsView` saved queries (deduped on save), `ModelBreakdown` series (built from unique labels), `PublishModal` warnings (needs one oversized asset referenced by two URIs; not reported, too speculative).

## Areas not yet covered (next iteration)

- `lib/components/Terminal.svelte` beyond its teardown and timers (the WS frame handling at ~`:960-1090`, the session-switch Effect 2 at ~`:2800-2885`, and the park/evict engine cache at ~`:100-215`).
- The `ResultsGrid` / `GridView` inline-edit commit flow after `EditFlow.runReview` (`EditFlow.svelte.ts:703-760`).
- `database.reattach` poll loop (`:3543`).
- Kubernetes `monitor/*` (MonitorCluster, MonitorFleet, MonitorInsights).
- AWS views other than SQS and Logs; `design-hall` beyond LinksPanel and Lobby; `DocsAgentsView`.
- `assistant` store and pages; `insights` page poll.
- `api` module request execution and streaming (`apiStream.svelte.ts`).
- Settings pages (Channels, Providers, ContextSoul).
- About 370 remaining string-keyed `{#each}` blocks whose source uniqueness I did not trace. Start with `QueryBuilder` join column options and the `SftpBrowser` rows window (`e.name`).

## Findings

### ui-correctness-2-01: major: An aborted dashboard-tile run is recorded as a success, so the tile can spin forever and never re-run
- **File:**
  - `ui/src/modules/database/WidgetCard.svelte:60`: `if (isAbortError(e) || signal?.aborted) return true;`
  - `:63`: `finally { if (!signal?.aborted) loading = false; }`
  - `:72-85`: the manual-only branch of the run effect.
- **Evidence:**
  - `run()` reports an abort as `true`. The manual-only branch (`refreshSecs` 0/null) then does `.then((ok) => { if (ok) { ranKey = key; okAt = Date.now(); } })`. On the next activation, `const fresh = key === ranKey && okAt > 0; if (secs <= 0) { if (fresh) return; … }` skips the run.
  - The aborted run also never clears `loading`, because the `finally` is skipped when the signal is aborted.
  - Aborts are routine:
    - The effect's cleanup aborts whenever the Home space goes inactive (`active` prop, "False while the card is off screen").
    - `widgetGate` (`createLimiter(2)`, `lib/poll.ts:202-224`) rejects every QUEUED tile with an `AbortError` on that abort. With more than 2 tiles, most tiles are queued at any moment.
  - The polled branch has the same problem: `pollWhileVisible`'s run returns `true`, so `okAt` is set and the next mount gets `immediate: false`.
- **Failure:**
  1. A Home space has a DB dashboard box with several manual-refresh widgets.
  2. Switch to another space while the tiles are still running or queued.
  3. Switch back. Every aborted tile shows the loading placeholder (`{:else if loading && !result}`) and a spinning refresh icon permanently. Its query never runs again until the user clicks each tile's refresh button.
  - For auto-refresh tiles, the spinner and empty body last a whole cadence (at least the dashboard interval).
- **Fix:**
  - Return a tri-state from `run` (`'ok' | 'failed' | 'aborted'`), and set `ranKey`/`okAt` only on `'ok'`.
  - On abort, clear `loading` unless a newer run owns it (a per-card run token): `if (token === runToken) loading = false`.
  - For the poller, return `false`/skip so freshness is not stamped.

### ui-correctness-2-02: major: The connection-import preview crashes when two skipped entries share a name (multi-version DataGrip and DBeaver installs)
- **File:** `ui/src/modules/connections/ConnectionImportDialog.svelte:286` (`{#each warnings as w (w)}`). The warnings come from `crates/otto-connections/src/conn_import.rs:693`, `:1006`, `:1052` and `:1068`.
- **Evidence:**
  - The scanner walks every DataGrip IDE folder (`glob_dirs(&jb, "DataGrip")`) and every project `.idea/dataSources.xml`, and every DBeaver `workspace*` / project.
  - It `warnings.append(&mut w)` per file. Connections go through `dedupe(found)`, but warnings are never de-duplicated.
  - JetBrains copies `options/dataSources.xml` forward on every IDE upgrade. A data source whose JDBC URL can't be parsed therefore yields the identical string `"'X' has no parseable JDBC URL — skipped"` once per installed DataGrip version. The same applies to `"'X' has no host — skipped"` across DBeaver workspaces.
  - A duplicate key throws `each_key_duplicate` in production. As iteration 1 noted, the app has no `<svelte:boundary>`.
- **Failure:** Import from DataGrip on a Mac with DataGrip2024.1 and DataGrip2024.2 settings folders, with one unsupported or URL-less data source. The preview step throws while rendering, and the user cannot import any connection from that tool.
- **Fix:**
  - Key by index: `{#each warnings as w, i (i)}`.
  - Also de-duplicate in the scanner (`warnings.dedup()` after a stable sort, or a `HashSet` guard), matching `dedupe(found)`.

### ui-correctness-2-03: major: The PR merge modal crashes when two CI checks share a name
- **File:** `ui/src/modules/git/PrMergeModal.svelte:209` (`{#each checks.checks as c (c.name)}`). The checks are built in `crates/otto-git/src/providers/github.rs:1147-1165` and `:356-381`.
- **Evidence:**
  - `list_checks` returns one row per GitHub check run, with `name: vstr(run, &["name"])`, which is the bare job name.
  - It then `rows.extend(checks_from_statuses(..))`. That function de-duplicates only among statuses, not against the check runs.
  - GitHub's API does not prefix the workflow name. Two workflows triggered on `pull_request` that each have a `build` / `test` / `lint` job produce two runs named `build`. A commit status and a check run with the same context name also collide.
  - The keyed `{#each}` then throws `each_key_duplicate` (production too, no boundary).
- **Failure:** Open Merge on a PR whose head has two `build` jobs from different workflows. Rendering the modal throws as soon as the checks load, and the merge dialog, which is the readiness gate, is unusable for that PR.
- **Fix:** Key by index (`{#each checks.checks as c, i (i)}`). Optionally, make server rows distinguishable: include the check-suite or workflow name in `name`, or add a unique `id` (the check-run id or status context plus source) to `PrCheck` and key by it. If you change the contract, update `docs/contracts` and `types.ts` together.

### ui-correctness-2-04: major: A Mission Control backfill finishing after a workspace switch loads the old workspace's items into the new one
- **File:** `ui/src/modules/mission-control/MissionControlPage.svelte:228-240` (`runBackfill`) and `:147-188` (`reload`).
- **Evidence:**
  - `runBackfill` captures `const id = ws.currentId`, then `await missionControlApi.backfill(id); await reload(id);`.
  - `reload` takes a fresh ticket (`const seq = ++reqSeq`) and commits `summary`, `items`, `graph` and `needsApprovalIds` when `seq === reqSeq`. It never checks `id === ws.currentId`.
  - The workspace-switch effect (`:79-100`) bumps `reqSeq`, and the load effect starts `reload(B)`. When A's backfill then resolves, `reload(A)` takes the NEWEST ticket. As a result, B's response is dropped as stale and A's lands under B.
  - Backfill ("Re-derive the graph from every source") is the slow, manual path, so switching away while it runs is natural. A failing backfill also writes A's `err` into B's page.
- **Failure:**
  1. In workspace A, press Refresh (backfill).
  2. Switch to B before it finishes.
  3. B's Mission Control lists A's work items, summary tiles and graph until the next live tick for B. Selecting a row opens `WorkItemDetail` with `wsId = B` for an A item id.
- **Fix:**
  - In `reload`, bail before committing when `id !== ws.currentId`: check it alongside `seq !== reqSeq` after each await.
  - In `runBackfill`, only `reload(id)` / set `err` when `ws.currentId === id`.

### ui-correctness-2-05: minor: A failed or stopped page change leaves the pager offset on the page that never loaded
- **File:** `ui/src/lib/stores/database.svelte.ts:3264-3270` (`runPage`). It is read by `QueryEditor.svelte:1558` → `ResultsGrid.svelte:248-249` and `:1723`.
- **Evidence:**
  - `runPage` assigns `t.offset = next` BEFORE `runQuery`.
  - `runQuery`'s abort/stop path ("A user-initiated abort isn't an error — leave the prior result intact") and its `ApiError` path both return without restoring the offset.
  - The footer computes `pageFrom = offset + 1`, `pageTo = offset + pageRowCount`, and the next `runPage` starts from the advanced offset.
- **Failure:**
  - Click Next on a slow auto-limited query, then Stop. The grid still shows rows 1–50, but the footer says "51–100".
  - The next Next requests offset 100, which skips a page. On a keyset Mongo find it sends the old page's `next_cursor` with offset 100, so page 2 is labelled 101–150.
  - A Next that errors does the same.
- **Fix:** Keep the previous offset (`const prev = t.offset`) and restore it when the run returns `null` without a new result (`if (!r && t.offset === next) t.offset = prev`). Alternatively, pass the offset in `opts` and commit it in `runQuery` only together with `installResult`.

### ui-correctness-2-06: minor: The Home DB-dashboard box reports a failed load as "No DB dashboards yet"
- **File:** `ui/src/modules/home/boxes/DbDashboardBox.svelte:44-50` and `:80-86`.
- **Evidence:**
  - `database.loadDashboards()` never throws. `fetchDashboards` catches and sets `database.dashboardsError` (`database.svelte.ts:4191-4192`), and a workspace change first empties `dashboards` (`enterDashboardsScope`).
  - The box only surfaces `database.connectionsError` (`:49`). Its `catch` is unreachable for dashboards.
  - With `loading` false, `loadError` empty and `dashboards.length === 0`, the template falls through to the EmptyState "No DB dashboards yet … Create one in Connections".
- **Failure:** The daemon errors on `GET …/dashboards` (or it times out). A user who has dashboards sees the box claim there are none, with a create CTA and no Retry.
- **Fix:** After `await database.loadDashboards()`, set `loadError = database.dashboardsError ?? loadError`. Alternatively, render `LoadState` from `database.dashboardsError` directly, with Retry calling `ensureLoaded(true)`.

### ui-correctness-2-07: minor: The multi-run preview crashes when two same-named targets produce the same warning
- **File:** `ui/src/modules/database/MultiRunDialog.svelte:530` (`{#each plan.warnings as w (w)}`). The warnings come from `crates/otto-dbviewer/src/service/multirun.rs:381-406`.
- **Evidence:**
  - Per-target warnings are formatted from `t.label` only: `"{label}: access-enforced production connection — its writes will be refused …"` and `"{label}: no ON CLUSTER — {note}"`.
  - `target_label` is `conn.name` (plus a scope). Connection names are not unique: there is no UNIQUE constraint in the migrations, and `ConnectionImportDialog.svelte:113` itself handles several saved connections with the same name and kind.
  - The duplicate-target check (`:221-229`) compares `connection_id`, not names.
- **Failure:** A multi-run over two connections both named `orders-prod` (say in two sections), both access-enforced, with a write statement. The preview step throws on render, so you cannot see or confirm the plan.
- **Fix:** Key by index (`as w, i (i)`). Optionally, include the target index in the server message, or de-duplicate the list.

### ui-correctness-2-08: minor: The Home Kubernetes box records an aborted first load as fresh and shows no monitor rows for up to a minute
- **File:** `ui/src/modules/home/boxes/K8sBox.svelte:63`, with `ui/src/modules/home/boxes/poll.ts:24-46` (`freshness`).
- **Evidence:**
  - `catch (e) { if (mine !== seq || signal?.aborted) return true; … }`. `freshness.run` stamps `at = Date.now(); atKey = key` on any `true`.
  - `freshness.start(key, 60_000)` then returns `immediate = false`.
  - The poller's signal is aborted whenever the box's Home space goes off screen (the effect cleanup calls `poller.stop()`).
- **Failure:** Leave the Home space while the first `monitorOverview` request is in flight, then come back within 60 s. `rows` is still `[]`, `loading` is false and `error` is empty. The box shows the registry clusters with no status or health data until the next 60 s tick.
- **Fix:** Return `false`, or a distinct "aborted" result that `freshness.run` does not stamp, when the signal aborted. This is the same root cause as 2-01; one shared helper rule for "aborted is not fresh" fixes both.

### ui-correctness-2-09: nit: Canvas `refreshSession` can attach the previous scene's agent session to the newly opened scene
- **File:** `ui/src/lib/stores/canvas.svelte.ts:257-264`. It is called from `MermaidCanvas.svelte:234`, `D2Canvas.svelte:231` and `ExcalidrawCanvas.svelte:265`.
- **Evidence:**
  - `const row = await api.get(\`/canvas/scenes/${this.currentId}…\`); this.sessionId = row.session_id;`. There is no check that `currentId` still equals the id requested.
  - `open()` uses `#openSequence` for exactly this purpose, but `refreshSession` does not.
  - `ConversationPanel.svelte:181-182` mounts `<LazyTerminal sessionId={canvas.sessionId}>` from that value.
- **Failure:** Right after an Ask-Otto turn finishes on scene A, open scene B before the GET returns. B's conversation panel shows, and accepts typing into, A's agent session. The window is a single GET.
- **Fix:** Capture `const id = this.currentId, seq = this.#openSequence` and assign only when `id === this.currentId && seq === this.#openSequence`.

## Rejected candidates (checked and dropped)

- **k8s `suspend()` leaves `rowsLoading` stuck:** no bug. ClusterWorkspace re-mounts and its load effect calls `loadResources()`, which owns a new controller and clears the flag. The access-revision path re-keys ClusterWorkspace (`KubernetesPage.svelte:162`).
- **`transcript.loadEarlier` leaving `loadingEarlier` true after an epoch bump:** no bug. The epoch bump resets it (`transcript.svelte.ts:530-533`).
- **`ws.loadArchived` leaving `archivedLoading` true after a workspace switch:** no bug. It is reset where the generation bumps (`workspace.svelte.ts:848-851`).
- **Vault `select()` racing a second `select()`:** no bug. Every loader is guarded by vault id and sequence, and the late `restoreView` only re-applies the current vault's persisted view.
- **`testConnection` result after a connection switch:** `testResult` is never rendered (snapshot-only), and the toast is generic. Not a user-visible bug.
- **SwarmSettings goals/triggers race:** the modal loads once per mount for the open swarm, and `detail` cannot change while it is open.
- **QuickOpen ArrowDown on an empty list (`index = -1`):** harmless. `aria-activedescendant` is guarded and `pick(undefined)` is a no-op.
