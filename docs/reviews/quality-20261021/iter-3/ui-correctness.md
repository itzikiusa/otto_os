# UI correctness review: iteration 3

Lens: logic bugs in the Svelte 5 UI (`ui/src/**`). This covers runes and reactivity, races between async loads and navigation, state left broken after errors, and API usage that does not match `ui/src/lib/api/types.ts`.
Branch `fix/quality-loop-20261021` @ `3dd55d9f2`. The review was read-only: no builds or tests were run. Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`, `ui/src/modules/home/boxes/ClassroomsBox.svelte`.

Svelte version check (`ui/node_modules/svelte` 5.57.1): `each.js:355-361` throws `each_key_duplicate` in production too, not only in DEV. The app has no `<svelte:boundary>` (`rg svelte:boundary ui/src` finds nothing). A duplicate key therefore breaks the whole rendered subtree.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 2 | 0.60 |
| minor | 4 | 0.40 |
| nit | 1 | 0.03 |

**Score = 10.0 - 0.60 - 0.40 - 0.03 = 8.97 / 10**

## Previous-finding status (iteration 2)

All 9 findings from iteration 2 are fixed in the current code.

| ID | Status | Evidence |
|---|---|---|
| ui-correctness-2-01 | Fixed | `WidgetCard.svelte:46-75`: `run()` returns `'ok' \| 'failed' \| 'aborted'`. Freshness is stamped only on `'ok'`. `finally { if (token === runToken) loading = false; }`. The poller returns `outcome !== 'failed'`. |
| ui-correctness-2-02 | Fixed | `ConnectionImportDialog.svelte:286` keys by index: `{#each warnings as w, i (i)}`. The scanner de-duplicates too (FIXES.md). |
| ui-correctness-2-03 | Fixed | `PrMergeModal.svelte:209`: `{#each checks.checks as c, i (i)}` |
| ui-correctness-2-04 | Fixed | `MissionControlPage.svelte:150`: `stale = seq !== reqSeq \|\| id !== ws.currentId`. `runBackfill` (`:234-242`) reloads and reports errors only `if (id === ws.currentId)`. |
| ui-correctness-2-05 | Fixed | `database.svelte.ts:3272-3297`: `const prev = t.offset`, then `.then((r) => { if (!r && outcome.status !== 'detached' && t.offset === next) t.offset = prev; })` |
| ui-correctness-2-06 | Fixed | `DbDashboardBox.svelte:76`: `error = $derived(loadError \|\| (database.dashboardsError ?? ''))` feeds `LoadState` with Retry. |
| ui-correctness-2-07 | Fixed | `MultiRunDialog.svelte:547`: `{#each plan.warnings as w, i (i)}` |
| ui-correctness-2-08 | Fixed | `K8sBox.svelte:63-66`: `if (signal?.aborted) return false;`, so `freshness` does not stamp it. |
| ui-correctness-2-09 | Fixed | `canvas.svelte.ts:257-266` captures `id` and `#openSequence`, and assigns only when both still match. |

Counts: 9 fixed, 0 still open, 0 rejected as invalid.

## Areas covered

- **Iteration-2 refactor regressions (`49ede5829`):** I diffed the high-risk splits against the code they replaced:
  - The event dispatcher moved to the typed route table (`lib/events/routes/*`). Every old `if/else` branch maps to a route, and the `ws.applyEvent` fallback is preserved.
  - The terminal frame decoder (`termProtocol.ts`) is wired into both the live engine and the parked engine. The binary-header-by-socket map, snapshot rule and credit path are equivalent.
  - The workflow-runs store was split out of `workspace.svelte.ts`. **Finding 3-06.**
  - The markdown renderer was consolidated into `lib/md`. **Finding 3-05**: WorkflowsPage lost `breaks: true`, while FileTree and RequestBuilder kept it.
  - `GitTabs` moved the tab role onto a button, and the query selectors still resolve.
  - `WipPanel` focus settling after a stage.
  - `CapsBanner` and the auth dismiss.
  - NotificationBell, HomeToday and SessionsBox moved to `rel()`.
  - Users and Plugins settings.
  - Import, Export and VerticalView inline validation.
  - `cadence.ts` uses the same weekday base (Mon=0) as the code it replaced.
- **Terminal:**
  - Effect 2 (session switch): timers, keepAlive park/adopt, `lastInjN` reset on both paths.
  - The parked-engine frame switch.
  - The restart-nonce effect and injection effect.
- **DB Explorer:**
  - The `EditFlow` review and commit flow (`runReview`) and its interaction with ResultsGrid's per-tab stash (`grid-tab-state`). **Finding 3-04.**
  - The `reattach` / `reattachLoop` poll: supersede guard, failure give-up, snapshot restore. Tab objects are shared by reference across connection snapshots. No defect found.
  - The QueryBuilder keyed-`{#each}` sources: group-by, issues, FK suggestions, columns. **Finding 3-01.**
  - `extractVars` de-duplication.
- **Kubernetes:** MonitorCluster (workloads, events and trends loads; abort, sequence and onDestroy), MonitorInsights (version guards), and the ResourceDrawer image list against its Rust producer. **Finding 3-03.**
- **AWS Athena:** catalog, run, poll backoff, history and `openExecution` generations.
- **Assistant store:** tickets, `turnReaders` acquire and release, `loadTurns` abort, pending-send settle, accounts sign-in fan-out, `applyEvent`.
- **Streaming API client:** `apiStream.svelte.ts` socket ownership, new-host confirm replay, the ring buffer and the rAF flush.
- **Settings > Channels:**
  - The load generation, the Slack directory `latestOnly`, and the status poll per workspace.
  - Save, toggle, allow and remove.
- **Design Hall:**
  - The `library.svelte.ts` load, patch and story resolution (**finding 3-07**).
  - ArtifactCard, ReferencesPanel and CollectionView story keys, traced to the `design_links` schema and `create_link`. **Finding 3-02.**
  - LinksPanel add-link.
- **Vault DocsAgentsView:** poll and liveQuery, start guard, retry, cancel, resolve, delete.
- **Insights page:** run and poll flow (stop, gone-ticks, ready path).
- **WorkflowsPage:**
  - The live-run sync: WS fast path, single-flight `refetchRun`, safety poll.
  - The final-output load.
  - `openRunById`, `startRun` ownership, auto-pick.
- **Keyed `{#each}` sweep:** I swept the 155 blocks keyed by the item value itself, plus the string-keyed blocks. I traced the server- and agent-fed sources:
  - **Confirmed (findings 3-01, 3-02, 3-03):** QueryBuilder group-by and issues, Design Hall story keys, k8s images.
  - **Checked and not reported:**
    - De-duplicated at the source: k8s monitor `versions` (`health.rs:184` `contains`), insights action `ids` (`Set`) and `statuses`, skill-eval matrix prompt labels (unique-validated in `eval_lab_routes.rs:256`), SQL vars (`extractVars` Set).
    - Unique or index-keyed by construction: `CsvPreview` (index columns), SFTP rows (unique dir entries), CreatePr reviewers (guarded on add).
    - AccessGroups members: the list buttons are disabled while busy.

## Areas not yet covered (next iteration)

- `lib/components/Terminal.svelte` WS reconnect and guest-refusal ladder (`connect()` at ~`:2400-2490` and `scheduleReconnect`), plus the `TermPark` adopt path (`switchEngine`).
- The `GraphView` (git) lane splice and refresh path after the iteration-2 `graph-lanes.ts` extraction (`graph-splice.ts`, `graph-cache.ts`).
- Product module: `OverviewTab` (25 changes in 3 weeks), `DesignArena`, `RewriteTab`, `TestCasesTab` (all changed in iteration 2), and `RefineChat` / `DiscoveryChat`.
- `ScheduledTasksPage`, `ProofPage` and `HistoryPage` (each changed 20+ times in 3 weeks).
- `apiClient.svelte.ts` (run/automation polling, `noteRunProgress`), and RequestBuilder.
- Swarm `KanbanBoard`, `RunInspector`, `SwarmPage`.
- MCP `AuditTab`, PoliciesTab.
- Personal-agents `RoomsView`, `PersonalAgentsPage`.
- The remaining string-keyed `{#each}` blocks fed by forge APIs:
  - Bitbucket `approved_by` / `reviewers` keyed by `display_name` (`PrDetail.svelte:545,566`).
  - GitLab reviewers keyed by `user.name`.
  - `MemoryTab` `m.tags`.
  - `NeedsYouCard` `ny.options`.
  - `ToolStep` `image_ids`.
  - `TurnItem` `parts.images`.

## Findings

### ui-correctness-3-01: major: The Query Builder crashes when the same column is selected twice and an aggregate (or "Group by them") is added
- **File:**
  - `ui/src/modules/database/QueryBuilder.svelte:1162`: `{#each clauses.groupBy as r, i (refKey(r))}`
  - The duplicate refs come from `:416-419` (`addColumnItem`), `:454-458` (`autoGroup`) and `:1264` ("Group by them").
  - `ui/src/modules/database/builder/sql-builder.ts:493-503` (`validate` → `loose`) and `:534-540` (`suggestGroupBy`).
- **Evidence:**
  - The select list may hold the same column twice:
    - `addColumnItem` always appends `firstRef()`, which is `colOptions[0]` (the first column of the first table), with a fresh `id`.
    - Clicking "+ Column" twice gives two `o.id` items. So does ticking `o.id` on the card (`toggleCol`) and then clicking "+ Column".
  - `suggestGroupBy` maps every plain column item to its `ref` and filters only those already in `groupBy`. It never de-duplicates among the select items, so two `o.id` items yield `[o.id, o.id]`.
  - `addAggregate()` → `autoGroup()` assigns `clauses.groupBy = suggestGroupBy(q)`.
  - The issue fix button does `clauses.groupBy = [...clauses.groupBy, ...refs]` with `refs = loose`, which has the same duplicates.
  - The chips block keys by `refKey(r)` (`"o.id"`) and throws `each_key_duplicate` (production too; see the header).
  - Same file, `:1258`: `{#each issues as is (is.message)}`. `validate` pushes the identical warning `"SUM of x (varchar) — not a numeric column."` once per aggregate item. Two SUM/AVG items on the same non-numeric column also crash.
- **Failure:**
  1. Open the Query Builder on a table.
  2. Tick its first column, then click "+ Column" (it adds the same first column), then click "+ Aggregate".
  3. The builder pane throws while rendering the GROUP BY chips and stops updating. The user loses the query being built.
- **Fix:**
  - In `suggestGroupBy` (and for the `loose` refs) de-duplicate by `refKey`: `[...new Map(refs.map((r) => [refKey(r), r])).values()]`.
  - Make the "Group by them" handler skip refs already in `clauses.groupBy` (the same guard `addGroupBy` has at `:510`).
  - Defensively key the chips and issues by index (`as r, i (i)` / `as is, i (i)`), or de-duplicate `issues` by message.

### ui-correctness-3-02: major: An artifact linked to the same story twice (for example `implements` plus `references`) crashes the Design Hall lobby, collections and References panel
- **File:**
  - `ui/src/modules/design-hall/ArtifactCard.svelte:59`: `{#each stories.slice(0, 2) as key (key)}`
  - `ui/src/modules/design-hall/ReferencesPanel.svelte:194`: `{#each h.story_ids.slice(0, 2) as sid (sid)}`
  - `ui/src/modules/design-hall/library.svelte.ts:97-101` (`storyKeys`, no de-dupe) and `CollectionView.svelte:193-194`.
  - The producer is `crates/otto-design/src/store.rs:263-264`: `group_concat(l.dst_id …) FROM design_links l WHERE l.src_artifact_id = a.id AND l.dst_kind = 'story'`. It has no `DISTINCT` and no `rel` filter, although `story_ids_for` (`:1765`) documents "stories an artifact `implements`".
- **Evidence:**
  - The unique index is `idx_design_links_uniq ON design_links(src_artifact_id, origin, rel, dst_kind, dst_id, src_node, dst_node)` (`migrations/0139_design_graph.sql:120`). The same artifact→story pair may therefore exist once per `rel` and once per `origin`.
  - A design created from a story gets an explicit `implements` link (`service.rs:652-665`).
  - LinksPanel's "Add link" lets the user target a story, with the rel defaulting to `'references'` (`LinksPanel.svelte:92`, `:153-162`).
  - `create_link` accepts any rel in `RELS` for `dst_kind = 'story'`.
  - `story_ids` then contains `[S1, S1]`. `storyKeys` maps both to `"LOY-142"`, and both land in `slice(0, 2)`.
- **Failure:**
  1. Open a design that was created from story LOY-142.
  2. Use Links → Add link → Story → LOY-142 (the default rel is "references"). It saves fine.
  3. Return to the Design Hall lobby. Rendering that card throws `each_key_duplicate`, and the lobby grid breaks on every visit while that link exists. The same happens in the project collection view and the References panel.
  - `model.ts:589` (the epic strip) also counts the artifact twice under the story.
- **Fix:**
  - Server: `group_concat(DISTINCT l.dst_id …)` and, per the documented contract, `AND l.rel = 'implements'`. Keep `story_ids_for` consistent.
  - UI: de-duplicate in `storyKeys` / `storyKeysOf` (`[...new Set(...)]`), and key the two chip loops by index.

### ui-correctness-3-03: minor: The Kubernetes resource drawer crashes for a pod whose containers reuse an image non-adjacently
- **File:**
  - `ui/src/modules/kubernetes/ResourceDrawer.svelte:319`: `{#each row.images as im (im)}`
  - The producer is `crates/otto-k8s/src/resources.rs:314-322`.
- **Evidence:**
  - `images()` collects `spec.containers[*].image` and then calls `out.dedup()`.
  - Rust's `Vec::dedup` removes only CONSECUTIVE duplicates.
  - A pod with containers `[web: app:1.4, proxy: envoy:1.30, worker: app:1.4]` (the same image running two commands, with a sidecar between them) keeps `app:1.4` twice. The keyed `{#each}` then throws.
- **Failure:** Open that pod, or its Deployment, in the Kubernetes console and select it. The drawer render throws, and the details pane for that workload cannot be opened.
- **Fix:** De-duplicate order-preservingly on the server (a `HashSet` seen-filter instead of `dedup()`), or `sort()` then `dedup()`. In the drawer, key by index (`as im, i (i)`).

### ui-correctness-3-04: minor: Inline edits that were already applied come back as "pending" when the post-apply refresh is skipped or fails
- **File:**
  - `ui/src/modules/database/EditFlow.svelte.ts:703-748` (`runReview`)
  - `ui/src/modules/database/ResultsGrid.svelte:296-315` (per-tab stash and restore)
  - `:1668-1680` (the pending bar)
- **Evidence:**
  - After `database.runManagedStatement` succeeds, `runReview` sets `this.reviewSql = null` but never clears `this.pending`.
  - Pending drafts are cleared only by `flow.resetForResult()` when a NEW result arrives (`ResultsGrid.svelte:336-339`). Two paths skip that:
    - **The tab changed during the write** (`:732-735`, "Applied in another tab", return). The tab switch already stashed `pending: flow.pending` (`currentGridState`). On return, `saved.result === resultProp` restores it (`:314`).
    - **The re-run fails or is stopped.** `runQuery` keeps the old result, so the effect never resets.
- **Failure:**
  1. Edit 3 cells, then click Review & apply. Switch query tabs while the write runs, or have the re-query error.
  2. On return the grid still shows "3 pending changes on 1 row — nothing is written until you review & run". Closing the tab asks to discard un-applied edits (`livePendingEdits`).
  3. "Review & apply" runs the same UPDATE a second time. On ClickHouse that is a second `ALTER TABLE … UPDATE` mutation.
- **Fix:** On a successful run of an edit review, clear the drafts it covered (`this.pending = new Map()`) before the refresh. Track that the open review came from `reviewPending()`, so delete and duplicate reviews leave pending alone.

### ui-correctness-3-05: minor: Workflow final-output reports lost their line breaks in the iteration-2 renderer consolidation
- **File:**
  - `ui/src/modules/workflows/WorkflowsPage.svelte:379-381`: `return renderMarkdownGfm(md);`
  - `ui/src/lib/md.ts:17-24`: `breaks: opts.breaks ?? false`
- **Evidence:**
  - Before `49ede5829`, the function was `marked.parse(md, { async: false, gfm: true, breaks: true })`.
  - The two sibling call sites converted in the same commit kept it: `FileTree.svelte` (`renderMarkdownGfm(src, { breaks: true })`) and `RequestBuilder.svelte` docs (`{ breaks: true }`). WorkflowsPage dropped the option, so `breaks` is now false.
  - `lib/md.ts`'s own doc says `breaks` exists for "agent reports … written line-by-line", and `final-output.md` is exactly such an agent deliverable.
- **Failure:** A run whose final output has line-by-line content (`Status: done` / `Files: 3` / `PR: …` on consecutive lines) now renders as one run-on paragraph in the Final output panel. The same file still renders correctly when opened from the file tree.
- **Fix:** `return renderMarkdownGfm(md, { breaks: true });`

### ui-correctness-3-06: minor: After a workspace switch the "Running" workflow list, nav count and Today rows show the previous workspace's runs (indefinitely if its session list fails)
- **File:**
  - `ui/src/lib/stores/workspace.svelte.ts:611-636` (`selectApproved`)
  - `ui/src/lib/stores/workflowRuns.svelte.ts:36-50` (`refresh`)
- **Evidence:**
  - `selectApproved` clears `sessions`, `otherWsSessions` and `openTabs` when the id changes, but not `workflowRuns`. It calls `refreshActiveWorkflowRuns()` only at `:634`, after `await refreshSessions` + `waitForSessions`. It skips the call entirely when `this.sessionsError` (`:633` returns first).
  - `workflowRuns.refresh` guards with `this.workspaceId === workspaceId`. That field changes only in `refresh`/`clear`, so an in-flight `refresh(A)` that lands after `currentId = B` (and before `refresh(B)` starts) is accepted, as is any refresh while B's sessions load.
  - The old in-store code compared against the LIVE `this.currentId`.
  - Readers that then show A's runs under B:
    - the sidebar Running list (`WorkflowsPage.svelte:2132-2138`)
    - the nav badge (`navBadge.ts:31`)
    - Home Today (`today.svelte.ts:210`)
    - the agent-facing `uiCommands/workflows.ts:77` `running` list
- **Failure:** Workspace A has a running workflow. Switch to B. The Workflows nav chip and Running list keep showing A's run until B's sessions finish loading. If B's session list fails, they keep showing it until a `workflow_run_updated` event for B forces a refetch.
- **Fix:** In `selectApproved`, when `this.currentId !== id`, call `workflowRuns.clear(id)` alongside the other per-workspace resets. Make `refresh` also drop a response when `workspaceId` is no longer the store's target, which `clear(id)` then guarantees.

### ui-correctness-3-07: nit: The Design Hall story-resolution stale-write guard is a no-op after the first load
- **File:** `ui/src/modules/design-hall/library.svelte.ts:269`: `if (my === this.seq || this.loaded) this.stories = next;`
- **Evidence:**
  - `loaded` becomes true on the first successful `load()` and never resets, so every `resolveStories` result is written, including a superseded one.
  - Two overlapping loads are routine: `refreshSoon` fires on design events.
  - The second load skips the workspace story list because `tried` already holds `ws:<id>` from the first load. It fetches at most 20 stories individually, builds `next` from the pre-first-load `this.stories`, and may finish last.
  - Its write then replaces the first load's complete map. The skipped ids stay in `tried`, so they are not retried until an events resync.
- **Failure:** Story chips and the epic strip lose labels for designs beyond the 20-id individual cap after a lobby open that coincides with a design event.
- **Fix:** Drop `|| this.loaded`, or merge into the live map instead of replacing it (`this.stories = { ...this.stories, ...next }`).

## Rejected candidates (checked and dropped)

- **MonitorCluster `loadWorkloads` `finally loading = false` on an aborted run:** a superseded run can clear `loading` while the newer one is in flight. With rows already shown the effect is invisible. With no rows yet it is a brief "No data yet" flash, too transient to count.
- **`apiStream` stale flush after `reset()`:** the queued rAF flush reads the NEW (empty) ring. Not a bug.
- **AccessGroups `membership` writing a stale group's members:** the group list is `disabled={busy}` during the mutation.
- **Workflows `loadFinalOutput` dropped then never retried:** needs an away-and-back switch to a non-success run within one GET. Too narrow to report.
- **k8s monitor `versions`, insights action `ids`, skill-eval prompt labels, SQL vars, CSV columns, SFTP names:** all de-duplicated or unique at the source (see Areas covered).
