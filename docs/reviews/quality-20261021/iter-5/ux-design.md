# Quality iteration 5 — lens: ux-design

Reviewer: ux-design. This review is static and read-only: no builds, tests or e2e were run.
Base: `fix/quality-iter5-20261022` @ `8177c4d85`.
Scope: all of `ui/src`. This now includes the room games (`modules/rooms/games/**`) and the classroom (`modules/home/school/**`, `modules/home/boxes/ClassroomsBox.svelte` and `classrooms/*`). Excluded: `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 10 | 1.00 |
| nit | 5 | 0.15 |
| **Total** | 16 | **1.45** |

**Score: 10.0 − 0.30 − 1.00 − 0.15 = 8.55 / 10.** Iteration 4 scored 9.15.

All 12 iteration-4 findings are fixed. I re-checked each one against the current code (table below). The lower score comes from two things. This pass covered areas no earlier pass reached: K8s Monitor, DB Dashboards, Diagram, Classroom, room games, MCP Profiles and Approvals, Workbench send-to, and the Home boxes. It also looked for cross-cutting patterns under the shared components: bare `{#await import()}` chunks, and `EmptyState` used as an error block. Most new findings are the same failure the earlier passes fixed one module at a time: a failed load that reads as empty, or a hand-rolled state.

## Previous-finding status (iteration 4)

| Id | Status | Evidence |
|---|---|---|
| ux-design-4-01 | Fixed | `SplitNode.svelte:213-231`: a Skeleton while the chunk loads. A failure shows `LoadState` with Retry (`dbLoad++`) and a "Reload Otto" `errorActions`. |
| ux-design-4-02 | Fixed | `SessionView.svelte:561` passes `(ms) => rel(ms)` to `suspendedNote`. |
| ux-design-4-03 | Fixed | The store has `automationsLoading/Error` (`apiClient.svelte.ts:342-343`), `runsError` (`:356`) and `cookiesLoading/Error` (`:984-995`). `AutomationsView.svelte:29-36`, `AutomationEditor.svelte:442-450` and `RequestBuilder.svelte:1233-1240` all use `LoadState`. |
| ux-design-4-04 | Fixed | `QueryBuilder.svelte:126` has `paletteError`, and `:912-913` uses `LoadState` with Retry. The import path says "Couldn’t load the tables of …" (`:831`), and `listBuilderTables` throws (`database.svelte.ts:2410-2412`). |
| ux-design-4-05 | Fixed | `AwsPage.svelte:244-254` checks `accountsError` before `!account`. `AccountRail.svelte:154-160` shows a Skeleton, then LoadState, then "No AWS accounts yet". |
| ux-design-4-06 | Fixed | `ConnectionForm.svelte:304-317` returns an inline error string. `:381-394` and `:506-517` keep the field open and add `aria-invalid`, `aria-describedby` and `.field-error`. |
| ux-design-4-07 | Fixed | The shared `ConfluencePagePicker` and `ConfluenceSpaceSelect` are used in PublishDialog, TestCasesTab and ProductPublish. The confirm copy says "under “{title}”" (`PublishDialog.svelte:140`, `TestCasesTab.svelte:763`, `WorkflowsPage.svelte:1475-1476`, `:2006`). |
| ux-design-4-08 | Fixed | `IdPicker.svelte` shows a disabled "Loading {what}…" select, uses free text only on error and takes an `emptyHint` prop (used by five inspectors). `connectionOptions` maps the engine and environment to words (`inspector/util.ts:218-223`). |
| ux-design-4-09 | Fixed | `BrowserPanel.svelte:668-674`: the button reads "Send N to {title}". It is disabled when no agent is open, with a reason. |
| ux-design-4-10 | Fixed | `RoomPage.svelte:113-116`: a pending guest is asked "Cancel your request to join?". |
| ux-design-4-11 | Fixed | Every listed site now says "Refresh" or names "Retry preview" (`WorkflowsPage.svelte:1463`, `PublishDialog.svelte:284`, `ArtifactView.svelte:482`, `SwarmSettings.svelte:314`, `DiscoveryTab.svelte:169`, `Channels.svelte:803`). |
| ux-design-4-12 | Fixed (listed sites) | No listed site still says "Could not". The only user-visible "Could not" strings left are in the room games, which earlier passes excluded (5-11). |

## Areas covered this iteration

- I re-verified all 12 iteration-4 findings (table above).
- **MCP (new feature):**
  - ProfilesTab: list/detail opens on the default profile, LoadState, delete confirm, copy fallback.
  - ProfileEditor: Modal, inline validation.
  - McpProfilePicker in NewSession, and SessionMcpProfiles: Save vs Save & restart, restart failure toast with action, viewer gating.
  - ApprovalsTab requester and decider labels.
- **Room games (first time in scope):**
  - GamesHub: PageHeader/PageBody, solo and room setup, create-room error.
  - GameMatch: lobby, invite copy, ready/start states, connection copy.
  - GameGuest: join form.
  - KartGame and ShooterGame error, paused and finished overlays.
  - Token use in game styles. Raw `rgba()` only in shadows, which ui-guards exempts.
- **Classroom (first time in scope):**
  - ClassroomsBox load, error, scene-error and empty states.
  - School actions: kick-out confirm copy and outcome, detention through `requestArchive`, release.
  - SchoolList rows, KidCard, NeedsYouQueue and `kidMenuItems`.
- **Kubernetes Monitor** (open since iteration 3): MonitorCluster, MonitorFleet, MonitorOverview, MonitorInsights and MonitorSettings (validation, test-probes modal). Also ClustersOverview, ResourceTable, MetricsView and InstallPanel.
- **Database:**
  - Dashboards and WidgetCard (open since iteration 3).
  - TableDesigner.
  - StructureView loading, error and empty states, plus its copy actions.
  - DiagramView load path.
  - ResultsGrid lazy dialogs.
- **Git:**
  - GraphView commit, branch, tag and stash context menus, with their confirms (revert, push tag, delete tag, drop stash).
  - PrMergeModal readiness rows and outward copy.
- **Skills Lab / Skills Eval** (open since iteration 3): GoldenTasksView, MatrixView, RunDetail, StartEvalForm, Scorecard, SkillsBrowser, SkillDetail, SkillEditor, SkillActivity, SkillReviewPanel and NewSkillModal. All states are designed.
- **Plugins** (PluginFrame: stopped, missing and loading states) and **Help** (Walkthroughs, TourFilm).
- **Browser module:** CredentialsPanel.
- **Design Hall:** site PublishModal.
- **Home boxes:** SessionsBox, DbDashboardBox, K8sBox, InsightsBox, UsageBox and MissionControlBox. I counted load and error handling in each.
- **Workbench:** send-to (Database "Run on…", Vault).
- **Cross-cutting greps:**
  - `{#await import(` with no `:catch`, across `shell/` and `modules/`.
  - `EmptyState` used as a load error (`title="Couldn’t …"`).
  - Clipboard-failure toasts.
  - Leftover `toastError('Couldn’t load …')` and the view each one feeds.
  - Raw user IDs (`decided_by`, `requested_by`).
  - "Could not".
  - Hex/rgba colours in rooms, home and mcp styles.

## Areas NOT covered yet (next iteration)

- Design Hall Site studio internals: Inspector, LeftPanel, SiteCanvas, and the assist panel (`assist/OttoPanel.svelte`, VariantsTray, TurnMessage).
- Brokers (Kafka viewer) message produce/consume flows and the ClusterForm beyond validation.
- Personal Agents and Mission Control detail flows. Only WorkItemDetail's `who()` was spot-checked.
- Vault NoteView, RefineDrawer and OpenApiView (agent-draft apply pattern), plus GraphView.
- The rest of Product: AnalysisTab, PlanTab and RewriteTab load-failure toasts (`AnalysisTab.svelte:314`, `PlanTab.svelte:147`, `RewriteTab.svelte:194`). I didn't trace what each view shows after the toast.
- Shooter and kart in-game HUD copy, and the game menus at phone width.
- Phone and tablet layouts of every module. This review is still static, with no screenshots.

---

## Findings

### ux-design-5-01: major. Lazy chunks rendered with a bare `{#await import()}`: no loading state, and a failed chunk is a silent, sticky no-op (New session, DB write review, Share and 18 more)

- **Files:**
  - `ui/src/shell/App.svelte`: `:921` (SharePage), `:1192`, `:1195`, `:1198` (Handover, AttachIssue, AttachProductStory), `:1202` (BroadcastModal), `:1206` (NewSession), `:1218` (NewWorkspace)
  - `ui/src/shell/Navigator.svelte:1093` and `ui/src/shell/TabBar.svelte:703` (ShareModal)
  - `ui/src/modules/database/ResultsGrid.svelte`: `:1798`, `:1802`, `:1806`, `:1810`, `:1822`, `:1835` (ReviewModal, the write review), `:1849`
  - `ui/src/modules/product/ProductPage.svelte`: `:852` (Learnings), `:920` (History), `:924` (Inject), `:941` (ImportDialog)
- **Evidence:**
  - Each site is `{#await import('…') then m}<m.default …/>{/await}`. None has a pending branch or a `{:catch}`.
  - `ui.newSessionOpen = true` (`Navigator.svelte:421`, `:462`, `:1221`) opens the sheet. If the chunk fails, nothing renders and nothing is announced. Because `newSessionOpen` stays true, every later "New session" click or ⌘N sets an already-true flag, so the app's main action stays dead until the window is reloaded.
  - `flow.reviewSql` (`ResultsGrid.svelte:1834-1846`) works the same way: the person clicks to review a write and no review appears.
  - The Product tabs render a blank pane while their chunk loads and stay blank if it fails.
  - The repo already has the fix for exactly this case:
    - `lib/lazy-component.svelte.ts:13-14`: "A failed load (offline, a stale chunk after an update) surfaces as `.error`; `retry()` clears it and loads again".
    - `components.md`, LazyMount row: "`quiet` is for overlays (nothing while loading, a toast on failure)".
    - Iteration 4 fixed the same failure in the split-view Database pane (4-01).
- **Fix:**
  - Hoist each import into a module-level `lazyComponent(() => import(…))`.
  - Render overlays and dialogs with `<LazyMount lazy={X} what="the new-session sheet" quiet props={{…}} />`.
  - Render the Product tabs with `<LazyMount … variant="panel">`.
  - For the overlays, also reset the open flag on failure (for example an `onerror` callback, or LazyMount's `quiet` toast with a "Try again" action), so the next click retries.

### ux-design-5-02: minor. `EmptyState` is used as a load-error block at 18 sites: no danger tile, no `role="alert"`, and not `LoadState`

- **Files:**
  - Agents: `ui/src/modules/agents/conversation/ConversationView.svelte:865`
  - AWS: `ui/src/modules/aws/EksView.svelte:159`, `RdsView.svelte:176`, `AthenaView.svelte:460`, `:608`, `Ec2View.svelte:238`, `S3Browser.svelte:590`, `:662`, `SqsView.svelte:376`
  - Kubernetes: `ui/src/modules/kubernetes/KubernetesPage.svelte:132`, `monitor/MonitorOverview.svelte:117`, `monitor/MonitorFleet.svelte:622`, `:643`, `:667`, `:747`, `:790`, `monitor/MonitorCluster.svelte:481`, `:521`
- **Evidence:**
  - Each one is `<EmptyState actionKind="secondary" icon="warning" title="Couldn’t load …" body={err} actionLabel="Retry" …/>` without `tone="error"`.
  - `EmptyState.svelte:27` and `:47-48` show that only `tone="error"` adds the danger icon tile and `role="alert"`. These failures therefore look like neutral empty states and are not announced.
  - `components.md`, "Error": "the `warning` icon on a `--danger` tile … `role="alert"`".
  - `components.md`, "The shared state components: use them, don't hand-roll an error block. A failed load inside an `{#if}` chain that already handles loading and empty is still `LoadState`".
  - Only 3 files in the whole UI pass `tone="error"`.
- **Fix:**
  - Replace each block with `<LoadState what="…" error={err} empty onretry={…} />`.
  - For the AWS "Sign in" branch, keep the sign-in button through LoadState's `errorActions` snippet.
  - Where a site must stay an EmptyState, add `tone="error"`.

### ux-design-5-03: minor. Database diagram: a failed load reads as "No schema selected — Pick a schema", with no Retry

- **Files:** `ui/src/modules/database/DiagramView.svelte:92-125`, `:441-442`; `ui/src/lib/stores/database.svelte.ts:2377-2389`
- **Evidence:**
  - `fetchSchemaGraph` catches every error, runs `toastError('Couldn’t load diagram', e)` and returns `null`.
  - `loadGraph` then sets `graph = null`, and the stage renders `<EmptyState icon="link" title="No schema selected" body="Pick a schema to draw its tables and relationships." />`. The schema select in the sidebar still shows a selected schema. The only way to retry is to re-pick the same value.
  - Iteration 4 fixed this pattern in QueryBuilder (4-04): `listBuilderTables` now throws, with this comment: "an empty list must only ever mean 'this database has none'". The diagram was not changed.
  - `components.md:491`: "A failed page load is an inline error, not a toast".
- **Fix:**
  - Make `fetchSchemaGraph` throw, or return `{ error }`.
  - In DiagramView, keep a `graphError` and render `<LoadState what="the diagram" error={graphError} empty onretry={loadGraph} />` before the `!graph` branch.
  - Keep "No schema selected" for an engine that really has no schema.

### ux-design-5-04: minor. Home "Agents" box shows "No open agent sessions" and 0 / 0 / 0 / 0 while sessions are loading or failed to load

- **File:** `ui/src/modules/home/boxes/SessionsBox.svelte:45-56`
- **Evidence:**
  - The box branches only on `sessions.length === 0`. It shows the stats row as all zeros and `<EmptyState title="No open agent sessions" body="Start Claude, Codex or a shell in this workspace.">` with a New session button.
  - The store already exposes `ws.sessionsLoading` and `ws.sessionsError` (`workspace.svelte.ts:133-134`, set at `:916-1014`). The box ignores both.
  - On a cold start, or when the session list fails, the dashboard tells the person they have no sessions and invites them to start one.
  - The sibling ClassroomsBox gates the same data correctly (`ClassroomsBox.svelte:628-634`).
  - `components.md:504`: "A failed load can therefore never read as 'No X yet'".
- **Fix:** Wrap the list in `<LoadState what="sessions" variant="compact" loading={ws.sessionsLoading && !ws.sessions.length} error={ws.sessionsError} empty={sessions.length === 0} onretry={() => ws.loadSessions()}>`. Move the existing EmptyState into `emptyView`, and show "—" in the stats while nothing is loaded.

### ux-design-5-05: minor. Workbench "Run on…": a failed connection load says "No database connections — Add one in Connections"

- **Files:** `ui/src/modules/workbench/send/sendTo.ts:56-66`; `ui/src/lib/stores/database.svelte.ts:1407-1450`
- **Evidence:**
  - `sendToDatabase` wraps `await database.loadConnections()` in `try/catch` and toasts "Couldn’t load connections" on a throw.
  - `fetchConnections` never throws. Its catch sets `this.connectionsError = loadErrorText(e)` and returns, so the catch in `sendToDatabase` is dead code.
  - After a failed load, `conns.length === 0` triggers `toasts.push('error', 'No database connections', 'Add one in Connections, then send the script again.', …, { action: { label: 'Open Connections' } })`. The person is told to create connections they already have.
  - `uiCommands/database.ts:135` already does this right: it checks `database.connectionsError` after the load.
  - `components.md:459-462`: show "not configured" only when there really are none.
- **Fix:** After `loadConnections()`, check `if (database.connectionsError && !database.connections.length)`. Toast "Couldn’t load your database connections" with `database.connectionsError` as the body and a "Try again" action that re-runs `sendToDatabase(ctx)`. Show the "No database connections" toast only for a successful empty list.

### ux-design-5-06: minor. MCP approvals show raw user IDs for "decided by", including your own, and for other requesters

- **File:** `ui/src/modules/mcp/ApprovalsTab.svelte:75-76`, `:204-210`, `:256`, `:266`
- **Evidence:**
  - `decided_by` is the deciding user's ID. The server calls `.decide(&id, req.approved, &user.id, …)` (`crates/otto-mcp/src/http.rs:1078`).
  - The tab passes it straight through: `<ApprovalOutcome … by={a.decided_by} …/>` and `` {a.decided_by ? ` by ${a.decided_by}` : ''} ``. `ApprovalOutcome.svelte:23` renders `` ` by ${by}` ``. Every decided approval therefore reads "Approved by 01J…", even when you decided it.
  - `requesterLabel` maps only your own ID to "you". It returns any other `requested_by` raw, so other people show as "requested by 01J… (MCP server)".
  - `content.md` §6: "Show the label, never the id". `review-checklist.md`: no ULIDs as primary text.
  - Mission Control already maps the same fields with `who()` = `ownerLabel(v, auth.me?.id) ?? shortId(v)` (`WorkItemDetail.svelte:304-307`, `:436`, `:449`).
- **Fix:** Add a shared `userLabel(id)`: "you" for `auth.me.id`, else the member's display name from the workspace members list, else `shortId`. Use it for `decided_by` and `requested_by` in ApprovalsTab, and keep the full ID in a `title`.

### ux-design-5-07: minor. Database dashboard tile: hand-rolled loading and error states (a static icon, a raw message with no headline or Retry)

- **File:** `ui/src/modules/database/WidgetCard.svelte:167-176`, `:52-73`
- **Evidence:**
  - Loading with no data is `<div class="wc-loading"><Icon name="refresh" size={14} /></div>`. That is a static glyph with no label, no `role="status"`, no spinner and no skeleton.
  - A first-load failure is `<div class="wc-error">{error}</div>`. The text is the raw message or the literal `'Query failed'` (`:65`), with no "Couldn’t run {title}" headline and no Retry in the body.
  - A failed refresh over data shows a pill that reads "refresh failed — showing last data", in lowercase. LoadState's wording for this state is "Couldn’t refresh {what} — showing the last good load".
  - `components.md` "Loading" and "Error", and "use them, don't hand-roll an error block".
- **Fix:** Render the body through `<LoadState what={widget.title} variant="compact" loading={loading && !result} {error} empty={!result} onretry={() => void run(true)}>…<Chart/></LoadState>`. LoadState already covers the stale-data bar, so the custom `.wc-stale` pill can go.

### ux-design-5-08: minor. Dashboard widget sheet: Add doesn't say which connection the widget runs on, and Edit offers columns from an unrelated query

- **Files:** `ui/src/modules/database/Dashboards.svelte:60-70`, `:92-96`, `:228-233`, `:250-268`; `ui/src/lib/stores/database/dashboards.svelte.ts:185-190`
- **Evidence:**
  - Add: the widget is bound to `this.host.selectedConnId()`, but the sheet shows the connection only when editing ("Runs on connection … (set at creation)", `:228-233`). The card's own comment admits the risk: "widgets bind to whichever connection was focused at creation" (`WidgetCard.svelte:122-123`).
  - With no connection selected, the form fills and submits, then fails with a bare `toasts.error('No connection selected')`. The guidelines say to disable the primary with a reason instead.
  - Edit: the X and Y selects use `resultCols = database.tab.result?.columns` (`:92-96`). That is the active Query tab's last result, not the widget's statement. Editing widget A while the Query tab holds some other result offers A's axes from the wrong columns. The hint "Run the query once in the Query tab to map specific columns" points the person at a tab that doesn't hold this widget's query.
  - `components.md:106-107`: disable the primary with a `title` that says why. Also the AGENTS.md rule to design every state.
- **Fix:**
  - On Add, show "Runs on {connection name}" (or a connection select). Disable "Add widget" with `title="Select a connection in Database Explorer first"` when none is selected.
  - On Edit, fetch the widget's columns (run `/db/widgets/{id}/run` with a small limit, or reuse the card's last result) and offer those. Change the hint to "Columns come from this widget's last run."

### ux-design-5-09: minor. K8s monitor settings: validation errors appear as a "Check the form" toast, and the inline messages aren't tied to their fields

- **File:** `ui/src/modules/kubernetes/monitor/MonitorSettings.svelte:159-163`, `:331`, `:336`, `:341`, `:346`, `:353`, `:387-389`, `:404`, `:433`
- **Evidence:**
  - `save()` runs `toasts.error('Check the form', Object.values(errors)[0])` whenever validation fails.
  - The inline errors are `<em class="err">…</em>` siblings. None of the inputs gets `aria-invalid` or `aria-describedby`.
  - The toast title doesn't say what failed. It duplicates only the first error and fades.
  - The form is long (probes and exclusions), and focus doesn't move to the first invalid field.
  - `components.md:100-106`: "Validation is inline … Don't use a toast for a field error … `<p class="field-error" id="…-err">` … `aria-invalid="true"` plus `aria-describedby`".
- **Fix:**
  - Drop the toast.
  - Render each error as `<p class="field-error" id="mon-{key}-err">` and give the field `aria-invalid` and `aria-describedby`.
  - On a failed save, focus and scroll to the first invalid field. Optionally add one summary line by the Save button: "Fix the 2 highlighted fields to save."

### ux-design-5-10: minor. Classroom list: clicking a kid in detention silently un-archives the session, and the visible row looks like "Open"

- **Files:** `ui/src/modules/home/boxes/classrooms/SchoolList.svelte:76-87`; `ui/src/modules/home/boxes/ClassroomsBox.svelte:501-506`; `ui/src/modules/home/school/actions.ts:117-127`
- **Evidence:**
  - Every row's main button calls `open(k)`.
  - For `k.detention`, `open` runs `await free(k)`, which un-archives the session (`ws.unarchiveSession`).
  - The visible row is the same as any other kid's (title, a small "detention" tag, state). Only the hidden `aria-label` says "Release …", and there is no `title`. A sighted person who clicks to look at an archived session restores it to a desk instead.
  - `release()` returns false without any feedback when `!s.canManage`, so for a viewer the click does nothing.
  - Guideline: a control's visible label names its action (`content.md` §6 vocabulary). Elsewhere the release is an explicit "Release from detention" button (`KidCard.svelte:39`, `helpers.ts:82`).
- **Fix:**
  - Make a detention row's main click open the session read-only (or select it and show its KidCard), and keep "Release from detention" as the explicit action.
  - Alternatively, label the row visibly: a trailing "Release" text button with `title="Bring this session back from the archive"`.
  - Disable the row with a `title` for people who can't manage the session.

### ux-design-5-11: minor. Room games: a failed "Create game room" reads "Couldn’t load game room", and the game copy still says "Could not"

- **Files:** `ui/src/modules/rooms/games/GamesHub.svelte:48`, `:134`; `ui/src/modules/rooms/games/GameGuest.svelte:11-12`
- **Evidence:**
  - A failed `POST /game-rooms` is shown with `<LoadState what="game room" {error} empty onretry={start} variant="compact" />`. LoadState's compact text is "Couldn’t load {what}." (`LoadState.svelte:66`), so a creation failure reads "Couldn’t load game room. …".
  - The fallback text is `'Could not create a game room. Try again.'`, which also breaks the voice.
  - The guest page throws `'Could not join this match. Ask the host for a fresh invitation.'` and `'Could not join the game.'`.
  - `components.md`: "a Retry for a failed *action* (a send, a save banner) is not a load error". `content.md:91-93`: "Write 'Couldn’t …', never 'Could not …'". The title should start with the verb the user tried.
- **Fix:**
  - Replace the LoadState with an action error: `<p class="field-error" role="alert">Couldn’t create the game room. {error}</p>`. The Create button stays the retry.
  - Change the GameGuest strings to "Couldn’t join this match. Ask the host for a fresh invitation." and "Couldn’t join the game."

### ux-design-5-12: nit. Clipboard-failure toasts don't say what to do next (8 sites)

- **Files:**
  - `ui/src/modules/database/StructureView.svelte:84`, `:163`
  - `ui/src/modules/database/ResultsGrid.svelte:1159`, `:1188`
  - `ui/src/modules/database/ErrorPanel.svelte:83`
  - `ui/src/modules/database/QueryBuilder.svelte:797`
  - `ui/src/modules/swarm/RunInspector.svelte:115`
  - `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:1383`
- **Evidence:**
  - Each one is `toasts.error('Couldn’t copy')`, `"Couldn’t copy to the clipboard"` or `"Couldn’t copy the report"`, with no body.
  - `content.md:100`: "Clipboard failures say what to do next ('Select the text and copy it manually')". Sibling sites already follow it: `CredentialsPanel.svelte:178`, `ProfilesTab.svelte:126`.
- **Fix:** Add the body "Select the text and copy it manually." Better, route these through the shared `copyWithToast` (`database/results-format.ts`) so the wording lives in one place.

### ux-design-5-13: nit. Site export and preview publish errors show the raw exception with no headline

- **File:** `ui/src/modules/design-hall/site/PublishModal.svelte:88`, `:100`, `:117`, `:170`
- **Evidence:**
  - All three actions set `error = e instanceof Error ? e.message : String(e)`. The banner renders `<p class="banner bad" role="alert">{error}</p>`, so it shows just the raw message with no "Couldn’t export the site" or "Couldn’t publish the preview".
  - `content.md:84-90`: the title is what failed, starting with the verb, and is never the raw exception. Raw detail goes last and dim. `loadErrorText` is the shared cause formatter.
- **Fix:** Store `{ what, detail: loadErrorText(e) }` and render "Couldn’t export the site." / "Couldn’t publish the preview." / "Couldn’t open the preview." with the detail on a dim second line.

### ux-design-5-14: nit. MCP Profiles: "Make default" and the ⋯ menu rows are disabled for non-admins with no reason

- **File:** `ui/src/modules/mcp/ProfilesTab.svelte:112-116`, `:180`
- **Evidence:**
  - "New profile" explains its disabled state (`title={isAdmin ? undefined : 'Needs MCP admin'}`, `:138`).
  - "Make default" uses `disabled={!isAdmin || busy}` with no `title`.
  - The menu rows Edit, Duplicate, Make default and Delete get `disabled: !isAdmin` with a `title` only for the locked Full profile.
  - `components.md:111`: "Explain disabled controls".
- **Fix:** Pass `title: 'Needs MCP admin'` for `!isAdmin` on each menu row and on the Make default button. Keep the existing locked-profile titles.

### ux-design-5-15: nit. Git graph menus: "Revert commit…" is painted destructive, and "Delete tag on origin" skips the outward confirm its sibling uses

- **File:** `ui/src/modules/git/GraphView.svelte:1243`, `:1260-1264`, `:1618-1636`
- **Evidence:**
  - The menu row is `{ label: 'Revert commit…', danger: true }`. Its own confirm says it is not destructive: `danger: false`, "This creates a new commit undoing its changes". `components.md`, ctxMenu table: `danger` = "Destructive styling".
  - "Push tag to origin" goes through `confirmOutward({ verb, where, who: 'Everyone with access to the remote sees the tag; CI or release jobs may react to it.' })`.
  - "Delete tag on origin…" asks only "Delete tag “x” on origin?". It is just as outward-facing, since it removes the tag for everyone and can trigger the same CI, but the confirm doesn't say who is affected (AGENTS.md: outward-facing actions confirm where and who).
- **Fix:** Drop `danger` from the revert row. Build the remote tag delete with `confirmOutward({ verb: 'Delete tag', where: 'origin · tag x', who: 'Everyone with access to the remote loses the tag; CI or release jobs may react.' })`, or add the same "who" sentence to the existing confirm.

### ux-design-5-16: nit. Kubernetes resource table: filtered-empty has no "Clear filter"

- **File:** `ui/src/modules/kubernetes/ResourceTable.svelte:112-115`
- **Evidence:**
  - When the text filter hides every row, the table shows `<EmptyState title="No rows match the filter" body="{total} hidden by the filter." />`, with no action.
  - `components.md:459-460`: filtered empty is "'No workflows match 'deploy'' plus 'Clear filter'". Iteration 3 fixed the same gap in MCP AuditTab (3-04).
- **Fix:** Add an `onclearfilter` prop that ClusterWorkspace wires to its filter state. Render `actionLabel="Clear filter"` and name the query in the title: "No {kind} match “{filter}”".
