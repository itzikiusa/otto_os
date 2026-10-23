# UI correctness review: iteration 6 (final)

Lens: logic bugs in the Svelte 5 UI (`ui/src/**`). This covers runes and reactivity, races between async loads and navigation, state left broken after errors, and API usage that does not match `ui/src/lib/api/types.ts` and the Rust producers.

Branch `fix/quality-iter6-20261023` @ `2065f00e9` (all iteration-6 fix waves). The review was read-only: no builds, tests or e2e were run. The scope was `git diff origin/main -- ui/src` (422 files), plus the wave-0 "not covered" list.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 3 | 0.30 |
| nit | 0 | 0.00 |

**Score = 10.0 - 0.30 = 9.70 / 10**

## Previous-finding status (wave 0)

All 5 wave-0 findings are fixed.

| ID | Status | Evidence |
|---|---|---|
| ui-correctness-6-01 | Fixed | `lib/stores/database/assist.svelte.ts`. `retireTurn` (`:190-201`) bumps `gen` on open and close, and records an orphan slot for a turn whose id is unknown. A stale POST response is discarded with a DELETE (`:153-156`, `discardOrphan` `:217-224`), and `finally` clears `assistBusy` only for its own generation (`:172`). An orphan's `db_assist_session_started` is DELETEd on sight (`setAssistSession` `:228-232`). Stop keeps the turn adoptable through `awaitingId` (`:179`), so Close can DELETE it. A residual case is not filed: a reopened turn's session event would have to overtake the orphan's, which already had a head start of several seconds. |
| ui-correctness-6-02 | Fixed | `modules/home/boxes/classrooms/load.svelte.ts:20-42`. An aborted read returns `false`, so `freshness` does not stamp it fresh, and `loaded` stays false (`:32-40`). |
| ui-correctness-6-03 | Fixed | `modules/product/MockupViewer.svelte:97-180`. A live edit of the diagram on screen is debounced (`scheduleRender`, 200 ms) and keeps the last good SVG (`live`, no `rendering` flip). `renderMermaid` gets `isStale`. `catch` and `finally` are guarded by the generation, and a mid-edit parse error is shown inline (`liveError`). `MockupAnnotations` stays mounted. |
| ui-correctness-6-04 | Fixed | `lib/components/MetricChart.svelte:327,349,362,374`: every `series` loop is keyed by index. |
| ui-correctness-6-05 | Fixed | `modules/kubernetes/monitor/MonitorSettings.svelte:137-139`. `revalidate()` runs after `removeProbe`, `removeMapping` and `removeExclusion` (`:228,252,287`), through the pure `revalidateAfterRemove` (`monitor-settings.ts`). |

Counts: 5 fixed, 0 still open, 0 partially fixed, 0 rejected as invalid.

## Areas covered

- **Re-verification** of all 5 wave-0 findings (table above).
- **Store and component splits made in this iteration.** I checked each split against the `origin/main` original with a normalized line diff (`this.db.`/`this.host.`/`h.` folded to `this.`, then each moved line looked up in the original). For every split, the only unmatched lines were the host contracts, the delegating façade members and the documented new behaviour.
  - **Database store split** (`lib/stores/database/{connections,schemaTree,objectOps,queryRuns,workbenchPersist,filters}.svelte.ts`). `ConnEpochs.of/live/bump` matches the old `epochOf`/`connLive`/epoch bump exactly, including the access-reset bump (`database.svelte.ts:483`). The plain (non-`$state`) fields that moved (`snapshots`, `restoring`, `restoreTombstones`, `runControllers`, `objectSearchSeq`, `warming`) were plain before. The sub-store constructors read nothing that the façade initialises later.
  - **`ConnSnapshot` capture and restore.** I compared the captured fields with the per-connection working set (`connections.svelte.ts:180-217`). `objectSearchScope` is a global UI preference, not per connection.
  - **apiClient split** (`lib/stores/apiClient/{tabs,drafts,execute,automations,cookies,collectionsSync,curl,model}`). This includes `ApiTabs` persistence and slots, and the `draft` getter and setter. The moves are faithful.
  - **Git graph split** (`modules/git/graph/graph{Data,Sync,Selection,Viewport,RefIndex,Actions,Menus}.svelte.ts`, `refDrag`, `refPopoverState`, and the GraphView wiring).
    - Loader effect `untrack`, `loadGen` and `inflight` release by identity, and the snapshot effect.
    - `refsRev` and bridge-refresh edge effects, the parked re-sync, and the background reload throttle.
    - Focus and deep-link effects (`revealSha` reads nothing tracked when the sha is already loaded).
    - Viewport window and lane memo, and host getters versus forward references (`vp`/`actions` are only called later).
  - **Workflows split** (`runSync.svelte.ts`, `runInputForm.svelte.ts`, RunMonitor, RunApprovals, RunContextSidebar).
    - `track()` is called during page init.
    - Approval-preview generation and abort, run-switch read abort, and the single-flight refetch.
    - The final-output effect reads only untracked bookkeeping, and `resetFinalOutput` on workflow open matches the old code.
  - **Product split** (`jiraIssue.svelte.ts`, `JiraIssueSection`, `StoryTranscripts`, `DraftEditor`, `caseOrder`).
    - `isCurrent` guards, `reset`, and the load and editmeta effects with their error gates.
    - The development-section lazy load through the toggle (the prop is read synchronously after the parent's reassignment).
    - Transcripts load once per mount, equivalent to the old `transcriptsLoaded`. **Finding 6f-01.**
- **New shared primitives:**
  - `lib/listWindow.svelte.ts` against the old `git/list-window` + `list-range`; the arithmetic is identical apart from the new `extra` band.
  - `lib/announce.svelte.ts`: effects write but never read the announcer, so they cannot loop.
  - `uiCommands/pagePort` with `vaultPort` and `homePort`: bind and unbind identity, and `ui_state` while unbound.
  - `LazyMount`/`lazyComponent` as used by ResultsGrid's 7 dialogs: `onerror` resets the open flags, and spread props stay reactive.
  - `lib/three/stage.ts` `demandLoop`, with the school scene: every `dirty` site outside the frame reaches `kick()` through its public caller.
- **Behaviour changes:**
  - DB edit review batches (`review-batch.ts`, `EditFlow.runReview`/`reportPartialReview`): canceled versus partial versus applied, `atomic` completed count, and the `stmtRows` mapping only while the text is unchanged.
  - AgentEditor identity gating against `otto-swarm` `hire_identity_fields`/`identity_changes`.
  - SessionMcpProfiles `mcp_skipped` keys: one row per enabled workspace server, unique by `UNIQUE (workspace_id, name)`.
  - AdminSessions paging: **finding 6f-03**.
  - FileTree APG keys: **finding 6f-02**.
  - SchemaTree on the shared window.
  - RoomPage, RecapPanel and RoomMedia reconnect and LoadState.
  - The Kart and Shooter HUD splits (Knocked, MatchResults, PauseMenu).
- **The a11y announce sweep** across about 40 files, checked for effects that read and write the same state. `wasStale`, `spokenRunError` and `wasWorking` are plain `let`s. No loops.
- **Wave-0 "not covered" list:**
  - Kart `net.ts`, traced field by field: 36 racer fields and 8 shell fields in matching order; global events use `racers[0]`, so decode never rejects them.
  - School scene loop.
  - WorkflowsPage.
  - Database `model.ts` snapshot users.
  - SwarmPage: AgentEditor and RunsList access only.
- **Sweep: lists keyed by value in changed files.** I traced each key to its producer. Git chips, branch leaves and stash refs are as before. `devStatus.*` keys combine the repo and the id. Jira `links`: **finding 6f-01**.

## Areas not covered

- `vault/GraphView.svelte` and `vault/DocsAgentsView.svelte` effects. Not changed this iteration; last reviewed in iteration 3.
- `workflows/WorkflowCanvas.svelte`. Not changed this iteration.
- `SwarmPage.svelte` beyond AgentEditor, RunsList and RunInspector.
- `design-hall/studio3d` (`embed.ts`, `sources.ts`, `loop.ts`) and the `site/Inspector` and `LinkListField` changes. These were skimmed only.
- Every remaining `{#each … (x.name)}` site in unchanged files (wave-0 swept about 80; this pass covered only changed files plus the Jira section).

## Findings

### ui-correctness-6f-01: minor: The Jira "Linked issues" list is keyed by issue key, but the producer emits the same key more than once, so the Details sidebar throws `each_key_duplicate`
- **File:**
  - `ui/src/modules/product/JiraIssueSection.svelte:297` (`{#each issueFull.links as lnk (lnk.key)}`)
  - Producer: `crates/otto-issues/src/jira.rs:1802-1862`
- **Evidence:**
  - `links` concatenates four sources with no de-duplication:
    - every `issuelinks` entry (`:1806-1831`);
    - `parent` as `"Parent"` (`:1834-1838`);
    - `customfield_10014` as `"Epic"` (`:1841-1853`);
    - every subtask (`:1856-1862`).
  - `JiraLink.key` is the linked issue's key (`jira.rs:188-196`).
  - The issue is read with `fields=*all` (`:1014`), so both `parent` and `customfield_10014` arrive.
  - Two ways one key appears twice:
    - A company-managed Jira Cloud story in an epic carries the epic in both fields: `{rel:"Parent", key:"EPIC-1"}` and `{rel:"Epic", key:"EPIC-1"}`.
    - Two links to the same issue with different types ("blocks" and "relates to", or an inward and an outward "blocks").
  - Svelte 5.57 throws `each_key_duplicate` in production too (`node_modules/svelte/src/internal/client/dom/blocks/each.js:355-361`), and the app has no `<svelte:boundary>`.
  - The loop moved here from OverviewTab in this iteration. It was not in wave-0's value-key sweep.
- **Failure:**
  1. Open Product, then a Jira story that is in an epic, or that links the same issue twice.
  2. Expand "Linked issues".
  3. The render throws, and the Details sidebar stops updating (status, assignee and fields freeze). The error goes to the console only.
- **Fix:**
  - Key by position: `{#each issueFull.links as lnk, i (i)}`. Alternatively key by `lnk.rel + ':' + lnk.key`, which is unique per relation.
  - Optionally, de-duplicate `Parent`/`Epic` with the same key in `jira.rs` (keep one row, `rel: "Parent"`).
  - Add a parse test with `parent` and `customfield_10014` naming the same epic.

### ui-correctness-6f-02: minor: Home/End (and any far jump) in a windowed side-panel file tree focuses nothing, and the focused row is then unmounted
- **File:**
  - `ui/src/modules/panels/FileTree.svelte:255-274` (`onRowKey` → `focusRow`)
  - `ui/src/lib/listWindow.svelte.ts:192-194` (`refresh` only schedules `#sync` on the next frame), `:220-234` (`reveal` scrolls but does not re-sync)
- **Evidence:**
  - `focusRow` does the following, in order:
    1. sets `activeRow`;
    2. calls `treeWin.reveal(i)`, which sets the scroller's `scrollTop`;
    3. awaits one `requestAnimationFrame` and a `tick()`;
    4. focuses `querySelectorAll('button.tree-row')[i - treeRange.start]`.
  - The window (`treeRange`) only moves when `#sync` updates `rel`. `#sync` runs from the scroller's `scroll` event, through `refresh`, which registers its own rAF. That rAF is queued after `focusRow`'s (lower handle, same frame), and `focusRow`'s `await` continuation and `tick()` run as microtasks in between. At the focus line the range is still the old one.
  - For a target outside the old slice (Home with `start > 0` gives a negative index; End gives an index past the mounted rows) the lookup is `undefined`, and focus does not move.
  - `#sync` then shifts the window, unmounting the row that held focus, so focus falls to `<body>`.
  - `ListWindow.findRows` already avoids this: its `reveal` calls `this.#sync()` right after `this.reveal(i)` (`listWindow.svelte.ts:266-268`).
  - Windowing starts above 300 visible rows (the default `min`).
- **Failure:**
  1. Side panel → Files: expand a large folder (more than 300 visible rows) and focus a row near the top.
  2. Press End (or Home after scrolling down).
  3. The list scrolls, but no row is focused and keyboard focus is lost. The roving tab stop has moved to the target (`activeRow`), so the user has to Tab back in.
- **Fix:** In `focusRow`, re-sync the window synchronously after `reveal(i)`. Expose a public `sync()` on `ListWindow` (the existing `#sync`) and call `treeWin.reveal(i); treeWin.sync(); await tick();`. Alternatively, after `tick()`, loop one more frame while `i` is outside `treeRange`.

### ui-correctness-6f-03: minor: Admin sessions paging is not invalidated by a reload, so "Include archived" during a Load more mixes two listings and keeps the old cursor; the load effect also tracks the filter
- **File:** `ui/src/modules/settings/AdminSessions.svelte`
  - `:55-57` (`$effect(() => { void load(); })`)
  - `:59-81` (`overviewUrl` reads `includeArchived`; `load` has no sequence guard)
  - `:84-97` (`loadMore` appends and sets `nextCursor` unconditionally)
  - `:99-102` (`setIncludeArchived` → `load()`)
  - `:238` (checkbox disabled only while `loading`, not `loadingMore`)
- **Evidence:**
  - **Effect tracking.** `load()` evaluates `overviewUrl()` synchronously inside the mount effect, so the effect now depends on `includeArchived`. A toggle re-runs the effect, and `setIncludeArchived` also calls `load()`: two identical first-page GETs per toggle.
  - **Paging race.** `loadMore` captures nothing about the listing it belongs to.
    1. Click "Load more" (unarchived page 2 in flight), then tick "Include archived". The checkbox is enabled, because only `loading` disables it.
    2. The reload replaces `sessions` with the archived-inclusive page 1.
    3. When the old page-2 response lands, it is appended (de-duplicated by id only), and `nextCursor` is overwritten with the unarchived listing's cursor.
    4. The next "Load more" sends that cursor with `include_archived=true`.
  - The same happens when a terminate or remove (which calls `load()`) lands while a Load more is in flight.
- **Failure:** The table shows page 1 of one listing followed by page 2 of another. Paging then continues from a cursor minted for the other query, which can skip or repeat rows until the page is reloaded.
- **Fix:**
  - Untrack the mount load: `$effect(() => { untrack(() => void load()); });`.
  - Add a `listing` generation, bumped by `load()`. `loadMore` captures it and drops its response, both rows and cursor, when it changed. `load()` also ignores a response that is not the newest.
  - Disable the checkbox while `loadingMore` too.

## Rejected candidates (checked and dropped)

- **Redis review drafts of refused rows cleared on "Applied"** (`EditFlow.clearCoveredDrafts`, `edit-redis.ts:300-309`). This is pre-existing. A refused row (line break in a field) cannot be written by this path at all, and the review shows its `#` note.
- **AgentEditor clears a recruiter prefill for a non-creator** (`AgentEditor.svelte:74-81`). This is intended: the server refuses identity fields on a non-creator hire (`otto-swarm/src/http.rs:304-330`).
- **`room-access.ts` importing `../../lib/rooms/invites.ts` with an extension.** This is a build or type concern, not a runtime logic bug, and it is outside this lens.
- **graphViewport `pulseTimer` not cleared on unmount.** It writes a `$state` of a disposed closure; nothing observes it.
- **DB assistant: a reopened turn's session event overtaking an older orphan's.** This needs the new turn to spawn faster than one started seconds earlier. Not realistic enough to file.
- **Announce-sweep effects re-announcing on every change of their source** (`TopicDetail` tail error, `MultiRunDialog` notice). Speech frequency belongs to the a11y lens. There is no state loop.
