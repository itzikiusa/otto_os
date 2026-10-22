# Quality iteration 4 — lens: ux-design

Reviewer: ux-design (static and read-only; no builds, tests or e2e were run). Base: `fix/quality-loop-20261021` @ `6cff08270`.
Scope excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`, `docs/reviews/`.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 7 | 0.70 |
| nit | 5 | 0.15 |
| **Total** | 12 | **0.85** |

**Score: 10.0 − 0.70 − 0.15 = 9.15 / 10.** Iteration 3 scored 9.37.

The drop does not mean the code got worse. Iteration 3 fixed 5 of its 7 findings fully and 2 partly. This pass reached modules that earlier passes did not cover: API-client automations, the cookie jar, the Query Builder, the AWS deep links, the connection form, Confluence publishing, the workflow ID picker, the browser panel and the rooms core. Every new finding comes from those areas.

## Previous-finding status (iteration 3)

| Id | Status | Evidence |
|---|---|---|
| ux-design-3-01 | Fixed | `runManagedStatement` returns `QueryResult \| ManagedWriteCanceled` (`database.svelte.ts:3367`). EditFlow toasts "Write canceled" only when the user declined (`res.canceled === 'declined'`, `EditFlow.svelte.ts:736-742`) and keeps the modal open. |
| ux-design-3-02 | Fixed | `workflows/inspector/IdPicker.svelte` is used by DbQuery, BrokerPeek, ProductStep, ManualTrigger and ProductPublish. Names are shown, an unknown id is kept as an option, and the field falls back to free text with Retry. The picker's own loading and empty states are new finding 4-08. |
| ux-design-3-03 | Fixed | `Navigator.svelte:122-141` `askClose` uses the shared `closeConfirm` builder (titles, mid-turn warning, singular/plural). Bulk archive asks when a selected session is mid-turn. |
| ux-design-3-04 | Fixed | `AuditTab.svelte:201` shows "No calls match these filters" with Clear filters. Line `:203` keeps "No calls yet" for an unfiltered empty list. |
| ux-design-3-05 | **Partly fixed, still open → 4-01** | The Usage half is fixed (`UsagePage.svelte:497`: real Retry, "Reload Otto" and an honest "Back to overview"). The split-view half is not: `SplitNode.svelte:213-217` is unchanged, and FIXES.md lists it under "Not done". |
| ux-design-3-06 | **Mostly fixed; remainder → 4-02** | GraphView `fmtDate` (`:2220`) and the product tabs (`NotesTab:114`, `QuestionsTab:310`, `AnalysisTab:400`, `TestCasesTab:600`) now call `rel()`. GraphView has an absolute tooltip. `suspendedNote` gained an `ago` parameter, but its only caller, `SessionView.svelte:555`, doesn't pass it, so the local ladder still runs. |
| ux-design-3-07 | Fixed (listed sites) | Rooms, Login and ProofPackView now say "Couldn’t". `toastTypedMismatch` (`confirmProd.ts:44`) is the shared mismatch toast. Other "Could not" strings remain in files that iteration 3 didn't list (4-12). |

## Areas covered this iteration

- I re-verified all seven iteration-3 findings (table above).
- **API client (iteration 3 hadn't covered it):** AutomationsView, AutomationEditor run history, the cookie jar in RequestBuilder, EnvironmentsView, CollectionsTree, HistoryList and the `apiClient` store load paths.
- **Database:** QueryBuilder table palette and query import, ResultsGrid empty/released/filtered states, GridView filtered-empty state, DbAssistantPanel run-write gate, and the DatabasePage hub empty pane (no auto-open on purpose).
- **Git:** WipPanel commit and agent-draft replace flow, and DiffViewer per-file pending/error/large states.
- **Connections:** ConnectionForm URI paste, SftpBrowser listing states and prod confirms. **Brokers:** ClusterForm validation.
- **Swarm:** KanbanBoard (LoadState, bulk confirm), OrgTree, BoardFeed, RunsList and RunInspector. **Loops:** GoalDefineForm validation (disabled-with-reason).
- **Personal Assistant:** AssistantPage, ChatView, TasksTab and MemoryTab. All states are designed.
- **Rooms core (non-games):** RoomPage, RoomLobby and the Recap panel copy.
- **Canvas:** CanvasPage list/detail, save-error banner and LoadStates. **Design Hall:** BrandKitPage, LearnedPage, ArtifactView phases, Studio3D and SiteStudio publish menu.
- **AWS:** AwsPage routing states, AccountRail, AccountsOverview and AthenaView.
- **Other modules:** Vault DocsAgentsView, browser panel (v1) vs Browser module SendToSession, and the workflow approval publication preview.
- **Cross-cutting greps:**
  - Load failures sent to toasts (`Couldn’t load` in `toasts.error`/`toastError`).
  - "No … yet" empty states in files with no loading state.
  - Raw-exception toast titles: none.
  - Bare "Loading…" regions: none.
  - "Could not" / "Failed to" copy.
  - "Reload" used for a data refetch.
  - Raw-ID input fields.
  - Pick-one empty panes.

## Areas NOT covered yet (next iteration)

- Kubernetes Monitor (`monitor/MonitorCluster.svelte`) and ClusterWorkspace after their recent changes. Iteration 3 covered only actions and the drawer.
- Skills Lab / Skills Eval (RunDetail, StartEvalForm), Plugins settings and Help/TourFilm.
- Design Hall Site studio internals (Inspector, LeftPanel, PublishModal) and the Otto assist panel (`assist/OttoPanel.svelte`, VariantsTray).
- Database TableDesigner, StructureView, Dashboards/WidgetCard and DiagramView.
- Git GraphView context-menu flows (rebase, cherry-pick, reset confirms) and PrDetail merge.
- Browser module (BrowserView, CredentialsPanel) beyond SendToSession.
- Phone and tablet layouts of every module. This review is still static, with no screenshots.

---

## Findings

### ux-design-4-01 — minor — The split view's Database pane, when it fails to load, shows the raw error with no Retry and no Reload (carried over from 3-05)

- **File:** `ui/src/modules/agents/SplitNode.svelte:213-217`
- **Evidence:**
  - `{:catch err}<div class="db-pane-loading" role="alert"><span>Couldn’t load the Database pane: {err instanceof Error ? err.message : String(err)}</span></div>`.
  - The raw chunk error ("Failed to fetch dynamically imported module: …") is the whole body, and there is no recovery control.
  - The Usage half of 3-05 was fixed with the shell pattern (`UsagePage.svelte:493-497`). FIXES.md lists this half under "Not done".
- **Fix:** Replace the block with `LazyMount` (`lazy={…} what="the Database pane" variant="panel"`). Alternatively, use `LoadState` with the shell wording, a Retry that creates a fresh `loadPage('database')` promise, and `errorActions` "Reload Otto", exactly as in UsagePage.

### ux-design-4-02 — nit — The session pane's "Suspended … ago" note still uses the local ladder (rest of 3-06)

- **Files:** `ui/src/modules/agents/SessionView.svelte:554-556`, `ui/src/lib/status.ts:347-353`, `:361-373`
- **Evidence:**
  - `suspendedNote(session?.meta, nowMinute())` is called without the new `ago` argument, so it falls back to `agoLabel`.
  - `agoLabel` returns "just now" / "Nm ago" / "Nh ago" / "Nd ago" with no date cutoff. The shared `rel()` returns "now", a date after about 30 days, and so on.
  - The doc comment on `AgoFormatter` says "The UI passes `rel` instead", but the only UI caller doesn't.
- **Fix:** Call `suspendedNote(session?.meta, nowMinute(), (ms) => rel(ms))` in SessionView (`rel` is already exported from `now.svelte`). Keep `agoLabel` only for the node-test fallback.

### ux-design-4-03 — minor — API client: a failed load of automations, run history or cookies reads as "No … yet"

- **Files:**
  - Store: `ui/src/lib/stores/apiClient.svelte.ts:1787-1801` (`loadAutomations`), `:1898-1907` (`loadAutomationRuns`), `:966-973` (`loadCookies`)
  - Views: `ui/src/modules/api/AutomationsView.svelte:29-38`, `ui/src/modules/api/AutomationEditor.svelte:443`, `ui/src/modules/api/RequestBuilder.svelte:1233-1234`
- **Evidence:**
  - Each loader catches the error, shows only `toasts.error('Couldn’t load …', loadErrorText(e))`, and leaves the list as `[]`. None of the three has a loading flag or an error field.
  - The views branch on `length === 0` alone:
    - AutomationsView renders `<EmptyState title="No automations yet" … actionLabel="New automation">` while loading and after a failure.
    - AutomationEditor shows "No runs yet." in both cases.
    - The cookie jar Modal shows "No cookies yet." in both cases.
  - The guidelines forbid this:
    - `components.md:410`: a pane that failed to load gets an inline error with Retry.
    - `components.md:491`: "A failed page load is an inline error, not a toast."
    - `components.md:504`: loading must show a skeleton, never the empty state, and "A failed load can therefore never read as 'No X yet'".
  - Sibling views in the same module already do this right: `EnvironmentsView.svelte:238-247` ("A failed/in-flight load is not 'No environments yet'"), `CollectionsTree.svelte:315` and `HistoryList.svelte:201`.
- **Fix:**
  - Add `automationsLoading/automationsError`, `runsLoading/runsError` and `cookiesLoading/cookiesError` to the store, set them in each loader, and drop the toasts.
  - Wrap each view in `LoadState` (`what="automations"` / `"run history"` / `"cookies"`, `onretry` → the loader).
  - Keep the existing EmptyStates as the `emptyView`.

### ux-design-4-04 — minor — Query Builder: a failed table list reads as "No tables in X" and "Table X was not found"

- **Files:** `ui/src/lib/stores/database.svelte.ts:2985-3020` (`listBuilderTables`), `ui/src/modules/database/QueryBuilder.svelte:289-296`, `:895-898` and `:812-822`
- **Evidence:**
  - On any error, `listBuilderTables` runs `toastError('Couldn’t load tables', e); return [];`.
  - The palette then renders `{:else if paletteTables.length === 0}<div class="pal-hint">No tables in {dbName(selectedDb) || 'this database'}.</div>`. A network or permission failure reads as an empty database, with no Retry; the only way to retry is to re-pick the database in the select.
  - The import path (`:818-822`) is worse. The same `[]` makes it say ``Table “${s.table}” was not found in ${want}``, so the user is told that a real table doesn't exist.
  - This breaks the same `components.md:504` rule as 4-03.
- **Fix:**
  - Have `listBuilderTables` throw, or return `{ tables } | { error }`.
  - In QueryBuilder, keep a `paletteError` and render `<LoadState what="tables" variant="compact" error={paletteError} empty onretry={loadPaletteTables} />` in the palette.
  - In the import path, report "Couldn’t load the tables of {db}" instead of "not found".

### ux-design-4-05 — minor — AWS deep link: a failed account list shows "Account not found … the link is stale" and "No AWS accounts yet"

- **Files:**
  - `ui/src/modules/aws/AwsPage.svelte:164-166` (`noAccounts` / `showRail`), `:244-254`
  - `ui/src/modules/aws/AccountRail.svelte:151-152`
  - `ui/src/lib/stores/aws.svelte.ts:196-212`
- **Evidence:**
  - When `listAccounts` fails, the store sets `accountsError` and `accountsLoaded = true`, and `accounts` stays `[]`.
  - With an account in the route (a reload or a pasted `#/aws/<id>/s3`), `noAccounts` is false, so the rail shows and AccountsOverview doesn't. AccountsOverview is the only component that renders `accountsError` with Retry (`AccountsOverview.svelte:99-107`).
  - The main pane falls through to `{:else if !account}<EmptyState title="Account not found" body="This AWS account was removed or the link is stale." …>`.
  - The rail's `{:else}` branch shows `<EmptyState title="No AWS accounts yet" />`.
  - So a transient failure tells the person that the account was deleted and that none exist. While the list is still loading, the rail also shows "No AWS accounts yet" next to the content Skeleton.
- **Fix:**
  - In AwsPage, test `aws.accountsError && !aws.accounts.length` before `!account`. Render `<LoadState variant="page" what="AWS accounts" error={awsErrorText(aws.accountsError)} empty onretry={() => aws.loadAccounts()} />`.
  - In AccountRail, render a Skeleton while `!aws.accountsLoaded` and nothing (or a compact LoadState) on error. Keep "No AWS accounts yet" for a successful empty list.

### ux-design-4-06 — minor — Connection form: an invalid pasted URI shows a toast, then closes and clears the field

- **File:** `ui/src/modules/connections/ConnectionForm.svelte:302-318`, `:339-341`, `:383-387`, `:492-505`
- **Evidence:**
  - `commitDsn()` always runs `parseDsn(dsnValue); dsnValue = ''; showDsnInput = false;`.
  - When `new URL(s)` throws, `parseDsn` toasts `'Invalid URI'` with "Could not parse as a connection URL — percent-encode / ? # @ in the password …". An unknown scheme toasts "Unknown scheme".
  - The pasted string, which usually contains a password, is then wiped and the input collapses. The person must reopen it, re-paste and hand-edit the value without seeing it next to the error.
  - This breaks `components.md:100-101`: "Validation is inline … Don't use a toast for a field error." It also breaks the title rule in `content.md:89` (start with "Couldn’t …") and the voice ("Could not").
- **Fix:**
  - Make `parseDsn` return an error string (or null).
  - In `commitDsn`, keep the field open and keep its value on error. Render `<p class="field-error" id="dsn-err">Couldn’t read this connection URL. Percent-encode / ? # @ in the password (for example %2F for /).</p>` with `aria-invalid` and `aria-describedby` on the input.
  - Clear and close only on success.

### ux-design-4-07 — minor — Confluence publishing asks for a raw "Parent page ID", and the outward confirm repeats the number

- **Files:**
  - `ui/src/modules/product/PublishDialog.svelte:406-416`, `:137`
  - `ui/src/modules/product/TestCasesTab.svelte:726-745`
  - `ui/src/modules/workflows/inspector/ProductPublish.svelte:55-70`
  - `ui/src/modules/workflows/WorkflowsPage.svelte:1998-1999` (approval preview)
- **Evidence:**
  - All three Confluence destinations take `parent_id` as free text: "Parent page ID (optional)", placeholder "e.g. 123456789".
  - The "where it goes / who sees it" sentences then say ``…space, under page ${parentId}…`` (`PublishDialog.svelte:137`) and `under page {approvalPreview.request.parent_id}` (WorkflowsPage), so the person approves a publish whose location is an opaque number.
  - The space field is also inconsistent:
    - PublishDialog offers a select of spaces loaded with `listConfluenceSpaces`.
    - TestCasesTab (`pf-space`) and the workflow inspector (`np-space`) ask the person to type the space key.
  - The client already has the search the picker needs: `lib/api/issues.ts:122` `searchConfluence({ accountId, q, space })`.
  - Guidelines:
    - `content.md` §6: "Show the label, never the id".
    - `review-checklist.md:148`: no ULIDs as primary text.
    - AGENTS.md: outward-facing actions confirm *where* the content goes.
  - Iteration 3 (3-02) fixed this same pattern for workflow node IDs.
- **Fix:**
  - Add one shared `ConfluencePagePicker` (a search-as-you-type typeahead over `searchConfluence({ space })`, showing page titles and storing the id) and a shared space `<select>`. Use both in all three forms.
  - In the confirm and approval copy, say "under “{parent title}”". Fall back to "under page {id}" only when the title can't be resolved.

### ux-design-4-08 — minor — Workflow ID picker: while loading it shows a raw-ID text field, and an empty list gives no explanation

- **Files:** `ui/src/modules/workflows/inspector/IdPicker.svelte:39-49`, `ui/src/modules/workflows/inspector/util.ts:217-219`; compare `SwarmTask.svelte:38` and `RepoPicker.svelte:26-27`
- **Evidence:**
  - `{#if options} <select…> {:else} <input type="text" {placeholder} …/>`.
  - `options` is null while the list loads (`cache.rows(key)` is null "while loading or after a failure"). So every time a node opens, the person first sees the free-text field the 3-02 fix was meant to remove ("DB-Explorer connection id", "Broker cluster id", "product story id"), and it then swaps to a select. Anything typed in the meantime is kept as an "Unknown connection (…)" value.
  - A loaded but empty list renders a select whose only option is "Choose a connection…", with no hint about where to create one. The sibling SwarmTask picker says "No swarms in this workspace yet — create one on the Swarm page."
  - Option labels are wire values: `` `${c.name} · ${c.kind} · ${c.environment ?? 'dev'}` `` gives "orders · mysql · prod". This breaks `content.md` §6: enums are mapped to words.
- **Fix:**
  - While loading, render a disabled select with "Loading {what}…", as RepoPicker does. Keep the free-text fallback only for `error`.
  - Add an `emptyHint` prop that renders a `node-hint` when `options.length === 0`, for example "No database connections in this workspace yet — add one in Database Explorer."
  - Map `kind` and `environment` through the shared engine label and EnvBadge wording.

### ux-design-4-09 — nit — Browser panel (classic): "Send N to agent" names no target and is enabled when no agent is open

- **File:** `ui/src/modules/panels/BrowserPanel.svelte:469-473`, `:661-666`; compare `ui/src/modules/browser/SendToSession.svelte:28-60`
- **Evidence:**
  - The button reads "Send {n} to agent".
  - When no agent session is active, it toasts "Can’t send yet". When one is active, it posts straight to `/sessions/{activeSessionId}/input` with `submit: true`, which starts a turn without naming the receiving session.
  - The Browser module's own SendToSession lists sessions by "title (provider)" and confirms with "Send this mark to {title} ({provider}) and start its next turn?".
  - `components.md:106-107`: disable the primary, with a `title` that says why.
  - This is a nit because the classic panel is opt-in (`ui.svelte.ts:228-229` defaults to v2).
- **Fix:** Label the button "Send {n} to {session title}". Disable it with `title="Open an agent session to send marks"` when no agent is active. Alternatively, reuse `SendToSession` so both panels pick and confirm the target the same way.

### ux-design-4-10 — nit — Rooms: "Cancel request" asks "Leave room?" and says access will close

- **File:** `ui/src/modules/rooms/RoomPage.svelte:155`, `:107-109`
- **Evidence:**
  - A guest who is still waiting for the host sees `EmptyState … actionLabel="Cancel request" onaction={leave}`.
  - `leave()` asks `title: 'Leave room?'`, `confirmLabel: 'Leave room'` and "Your terminal, chat and media access will close.", but a pending guest has no access yet.
  - Two guideline rules are broken: the confirm verb should repeat the action the person clicked, and the body describes a state this person isn't in.
- **Fix:** When `room?.admission === 'pending'`, ask "Cancel your request to join?" with confirmLabel "Cancel request" and body "The host won’t see your request any more." Or skip the confirm, since nothing is lost.

### ux-design-4-11 — nit — "Reload" used for data refetches, and copy that names a control the page doesn't have

- **Files and evidence:**
  - The vocabulary table (`content.md` §6) says: **Refresh** fetches the data again; **Reload** reloads the UI itself. These strings say "Reload" for a data refetch:
    - `modules/workflows/WorkflowsPage.svelte:1463`: "The publication preview changed. **Reload** and review it before deciding." The only control below it (`:1994`) is "**Retry preview**".
    - `modules/product/PublishDialog.svelte:281`: "**Reload the preview** and review it…". The button (`:439`) is "**Retry preview**".
    - `modules/design-hall/ArtifactView.svelte:482`: "The design changed meanwhile. **Reload** and try again."
    - `modules/swarm/SwarmSettings.svelte:314`: disabled reason "Reload the triggers first".
    - `modules/product/DiscoveryTab.svelte:169`: `title="Reload discovery runs"` on a button labelled "Refresh". The tooltip should equal the label.
    - `modules/settings/Channels.svelte:801`: `title="Reload names from Slack"` on "Refresh names".
- **Fix:** Use "Refresh" for these data refetches. Make the two preview messages name the real control: "…Select Retry preview and review it again before deciding."

### ux-design-4-12 — nit — "Could not …" copy remains outside the sites fixed in 3-07

- **Files and evidence:**
  - The app voice is "Couldn’t …" (`content.md:84-93`). These user-visible strings still say "Could not":
    - Canvas: `modules/canvas/CanvasPage.svelte:206` ("Could not save {title}…"), `D2Canvas.svelte:146`, `:184` and `MermaidCanvas.svelte:136`, `:187` (the toast body repeats its "Couldn’t copy" title as "Could not copy to the clipboard.")
    - Browser: `modules/browser/CredentialsPanel.svelte:178`
    - Agents: `modules/agents/FirstRunCoach.svelte:264`, `modules/agents/conversation/SubagentCard.svelte:74`
    - MCP: `modules/mcp/OttoServerHome.svelte:352`, `:515`
    - Other modules: `modules/snip/SnipEditor.svelte:126`, `modules/workbench/preview/CsvPreview.svelte:25`
    - Rooms clients: `modules/rooms/room-client.ts:20`, `modules/rooms/recap-client.ts:21`
    - `lib/components/FolderPicker.svelte:95`, `:239`
- **Fix:**
  - Change these strings to "Couldn’t …".
  - For the clipboard toasts, make the body say what to do ("Select the text and copy it manually."), as `ProofPackView.svelte:74` does, rather than repeating the title.
