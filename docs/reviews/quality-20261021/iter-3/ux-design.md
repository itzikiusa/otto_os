# Quality iteration 3 — lens: ux-design

Reviewer: ux-design (static, read-only; no builds, tests or e2e run). Base: `fix/quality-loop-20261021` @ `3dd55d9f2`.
Scope excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`, `docs/reviews/`.

## Score

Rubric: start 10.0; blocker −1.0, major −0.3, minor −0.1, nit −0.03.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 6 | 0.60 |
| nit | 1 | 0.03 |
| **Total** | 7 | **0.63** |

**Score: 10.0 − 0.60 − 0.03 = 9.37 / 10** (iteration 2: 9.04)

## Previous-finding status (iteration 2)

| Id | Status | Evidence |
|---|---|---|
| ux-design-2-01 | Fixed | `lib/confirmOutward.ts:92-103` `confirmRunAndDeliver` (where / what / who). It is used by `AgentPage.svelte:274` (all three Run buttons route through `runNow`), `PersonalAgentsPage.svelte:106` and `ScheduledTasksPage.svelte:668`. It returns true without a dialog only when nothing is delivered. |
| ux-design-2-02 | Partly fixed (the rest is 3-01) | Store gate `database.svelte.ts:3310-3349`: the title follows the environment, the glyph is gone, and a mismatch toasts the `confirmProd` wording and answers `'mismatch'`. ImportDialog's copy (`:183-206`) matches. **Still open:** `runManagedStatement` folds `'mismatch'` into `null`, and EditFlow then adds a second "Write canceled" toast (see 3-01). The "reuse one gate" half is deferred by FIXES.md and is not a visible defect. |
| ux-design-2-03 | Fixed | Inline `field-error` + `aria-invalid` + `aria-describedby` in CredentialsPanel (`:247-269`), RequestBuilder (`:1042-1045`), TestCasesTab (`:942-946`), ImportDialog (`:266`) and ExportDialog (`:272`). VerticalView uses `promptText({validate})` (`:259`, `:275`). The Channels toast at `:317` is the list-card quick toggle, which then opens the editor where the message is inline (`:831`). |
| ux-design-2-04 | Fixed | Shared `lib/cadence.ts` (`everyLabel`, `cadenceLabel`), used by AgentPage, TriggersPanel `describeSpec` (`:193-199`) and ScheduledTasksPage `:1266`. |
| ux-design-2-05 | Fixed | `TopicDetail.svelte:456-461` keeps the previous offset, or falls back to the first message after a consume. |
| ux-design-2-06 | Fixed (for the listed files) | HomeToday, SessionsBox, NotificationBell, FocusView (`:271` → `rel(…, {compact})`), DatabasePage and mission-control `relTime` (`lib.ts:181`) all use `rel()`. Other hand-built ladders remain (new finding 3-06). |
| ux-design-2-07 | Fixed | `shell/CapsBanner.svelte`: an in-flow strip above the status bar (`shell/App.svelte:1384`) with Retry and Dismiss (`auth.capabilitiesNoticeDismissed`). |
| ux-design-2-08 | Fixed | DesignArena `:797` uses `plural(…,'file')`. The sites that said "blender render", "postman sync", "row(s)", "unknown error" and "Could not create session" are gone. |
| ux-design-2-09 | Fixed | TriggersPanel `:176-189`: "Delete trigger" / "Trigger deleted" / "Couldn’t delete the trigger" with `KIND_LABEL`. VerticalView `:285-287` says "Delete “path” from this document?". |

8 of 9 are fixed. 2-02 is partly fixed and its remaining defect is reported as 3-01. None were rejected.

## Areas covered this iteration

- I re-verified all nine iteration-2 findings against the current code (table above).
- **Outward-facing confirms, cross-module:**
  - Every `api.post/put/patch` call that sends a comment, transition, merge, publish, send, deliver or share.
  - Product OverviewTab Jira transition and assign, RewriteTab publish and QuestionsTab post.
  - AWS SQS send/purge/delete (`confirmProd`), EKS import, account delete, Kubernetes `runAction` and agent-driven `lib/uiCommands/k8s.ts` and `scheduled.ts`.
  - ShareModal mint/revoke. All of these conform.
- **Page shell:** `PageHeader` + `PageBody` on every `modules/*/*Page.svelte`. The exceptions are nested hosts: SkillsEvalPage inside SkillsLabPage, CapabilitiesPage inside InsightsPage, and Design Hall/Kubernetes, whose sub-views own `PageBody`.
- **One-primary rule:** I scripted a count of `primary` per PageHeader block. Every hit with more than one (LoopDetail, AgentPage, SwarmPage, WorkflowsPage…) is a set of mutually exclusive `{#if}` branches.
- **List/detail opens on an item:** HistoryPage (restore, then newest), Mission Control, Swarm, Insights, Workbench (`workbench.svelte.ts:214-219`), DesignArena (`:317-319`), Vault landing note, Proof and Brokers GroupsTab. All conform. The empty "pick" panes show only on phone or with zero items.
- **Settings:** Users (load/error/empty, matrix and grants LoadStates, create Modal inline error), SecretsStore, Onboarding copy and errors.
- **Agents core:** HistoryPage states, the Composer send failure (keeps the draft), SessionView pane state, and shell Navigator bulk actions vs the Palette's English-command close.
- **Kubernetes:** ClustersOverview remove, ResourceDrawer loading/error per tab, `actions.ts` typed and prod confirms, the agent path.
- **Workflows:** the node inspector forms (every `selectedNode.kind` branch), generator, RunAgents and TriggersPanel.
- **Product tabs:** Chat, Refine and Discovery archive and run confirms, DesignArena selection fallback.
- **Other modules:** MCP AuditTab (filters and empty state), Usage page lazy "Otto usage" view, SplitNode lazy Database pane, Workbench.
- **Cross-cutting greps:**
  - `confirmer.ask` without `confirmLabel`: all are real deletes.
  - Hex colours: only allow-listed page canvases.
  - Fonts under 11 px: none.
  - Off-voice error copy ("Failed to…", "Could not…").
  - "No … yet" empty states in filterable lists.
  - Coordinate-positioned popovers: all clamp and cap their height.

## Areas NOT covered yet (next iteration)

- Canvas (Excalidraw/D2/Mermaid) editor states, and the Design Hall studios (Site, 3D, BrandKit, LearnedPage) in depth.
- Swarm KanbanBoard, OrgTree and run feed. Loops GoalDefineForm validation.
- Database ResultsGrid, GridView and QueryBuilder after the iteration-2 UI splits (heavily changed; only EditFlow was traced this pass).
- Git DiffViewer, WipPanel commit flow and ReviewPanel after the splits. Only the confirm copy was checked.
- API client (RequestBuilder body and auth tabs, EnvironmentsView, AutomationEditor run states).
- Connections (SSH/SFTP browser, ConnectionForm validation), Brokers ClusterForm, Personal Assistant threads.
- Rooms core (non-games): RoomPage, RoomLobby, Recap panels. Only the copy voice was sampled.
- Every module's phone and tablet layout. This is still a static review only, with no screenshots taken.

---

## Findings

### ux-design-3-01 — minor — A grid edit on a guarded connection shows two toasts when the typed name doesn't match

- **Files:**
  - `ui/src/lib/stores/database.svelte.ts:3379-3381` (`runManagedStatement`)
  - `ui/src/modules/database/EditFlow.svelte.ts:720-724` (`runReview`)
- **Evidence:**
  - `confirmGuardedWrite` toasts "Nothing was changed — What you typed didn’t match…" and answers `'mismatch'` (`:3343-3346`). Its doc comment says this is "so callers don't add a second, vaguer 'canceled' toast".
  - `runManagedStatement` collapses the answer to a boolean: `const ok = (await this.confirmGuardedWrite(origin, e)) === 'confirmed'; if (!ok …) return null;`.
  - EditFlow's review modal then does `if (res === null) { toasts.info('Write canceled'); return; }`.
  - Result: a near-miss typed name on a prod or read-only connection stacks "Nothing was changed" and "Write canceled". The two direct query paths (`:3169`, `:3183`) skip the second toast on `'mismatch'`, so the three write paths in the same store disagree.
- **Fix:** Have `runManagedStatement` return a discriminated result, for example `{ result } | { canceled: 'declined' | 'mismatch' | 'stale' }`, or throw a typed cancel. Then make EditFlow toast "Write canceled" only for `'declined'`, matching `:3169`.

### ux-design-3-02 — minor — Workflow node inspector asks for raw internal IDs that the UI never shows

- **File:** `ui/src/modules/workflows/WorkflowsPage.svelte`:
  - `:2804-2811` (`db_query` "Connection ID", placeholder "DB-Explorer connection id")
  - `:2830-2837` (`broker_peek` "Cluster ID", placeholder "Broker cluster id")
  - `:2655-2661` ("Story ID (for product steps)", placeholder "product story id")
  - `:2730-2737` ("Jira account ID")
- **Evidence:**
  - These fields are free-text inputs for opaque IDs. Validation requires them: `crates/otto-workflows/src/validation.rs:115-116` lists `connection_id` and `cluster_id` as required for these node kinds.
  - Neither the Database Explorer nor Brokers offers a "Copy ID" action (grep finds none), so a person building the node by hand has no in-app way to find the value.
  - The same inspector already does this properly for `swarm_task` (`:2893-2925`): it loads the workspace's swarms and projects into `<select>`s, with an "Unknown swarm (id)" fallback option. It falls back to a text input only when the list can't load.
  - Guidelines: `review-checklist.md:148` says no ULIDs as primary text, and `layout.md:328` says raw IDs don't belong in rows. Here the person is made to *type* them.
- **Fix:** Reuse the swarm-picker pattern:
  - `db_query`: a `<select>` of the workspace's DB connections (name · engine · env badge).
  - `broker_peek`: broker clusters, then a topic list when the cluster is chosen.
  - Story: product stories by key and title.
  - Jira account: `/issue/accounts` (label).
  - Each keeps an "Unknown … (id)" option for a stale value and the text input as the load-failure fallback.

### ux-design-3-03 — minor — Sidebar bulk Delete/Archive is weaker than the Palette's version of the same action

- **Files:**
  - `ui/src/shell/Navigator.svelte:121-137` (`archiveSelectedAgents`, `deleteSelectedAgents`), buttons at `:1319`, `:1322`; archived bulk delete at `:163-173`
  - Compare: `ui/src/shell/Palette.svelte:376-389`, `ui/src/lib/stores/workspace.svelte.ts:1664-1678`, `:1691-1696`
- **Evidence:**
  - Bulk **Delete** confirm: the title is `` `Delete ${ids.length} sessions` ``, so one selected session reads "Delete 1 sessions". The body is "Delete 1 session and their entire history?".
  - It doesn't name the sessions, and it doesn't warn that live ones stop mid-turn.
  - The Palette's close of the same sessions does all of these:
    - It lists up to five titles plus "…and N more".
    - It adds "N sessions are working right now and will stop mid-turn."
    - It picks the singular or plural title ("Delete session?" / "Delete sessions?").
  - Bulk **Archive** has no confirm and no Undo. `archiveSessions` toasts only "N sessions archived". The single-session archive offers Undo (`workspace.svelte.ts:1694-1696`), and the Palette asks first with the same busy note.
  - So two entry points for the same destructive operation behave differently. Archiving a running agent from the sidebar stops it silently.
- **Fix:** Reuse the Palette's message builder (titles, busy note, singular or plural title) for both Navigator deletes. For bulk archive, either confirm when any selected session is working, or give the bulk toast an Undo that unarchives the batch.

### ux-design-3-04 — minor — The MCP audit log says "No calls yet" when the filters match nothing

- **File:** `ui/src/modules/mcp/AuditTab.svelte:184-187`. Filters at `:157-180`, applied server-side by `load()` at `:102-108`.
- **Evidence:**
  - Server, tool and decision filters reload `rows` from the server. When a filter matches nothing, `rows.length === 0` renders `<EmptyState title="No calls yet" body="Calls made by external MCP clients … appear here.">`.
  - The filter bar sits behind a collapsible "Filters" toggle (`filtersOpen`), so the person may not even see that a filter is active.
  - `components.md:459-460`: "Distinguish **empty** ('No workflows yet' plus a CTA) from **filtered empty** ('No workflows match …' plus 'Clear filter')". The sibling list pages already do this (HistoryPage `:647-654`, MissionControlPage `:394-401`).
- **Fix:** Track `hasFilters = !!(fServer || fTool.trim() || fDecision)`. When it is true, show "No calls match these filters" with a "Clear filters" action that resets all three and reloads. Keep "No calls yet" for the unfiltered case.

### ux-design-3-05 — minor — A lazy view that fails to load offers a "Retry" that navigates away, or no recovery at all

- **Files:**
  - `ui/src/modules/usage/UsagePage.svelte:493`
  - `ui/src/modules/agents/SplitNode.svelte:212-218`
  - Reference pattern: `ui/src/shell/App.svelte:1229-1238`
- **Evidence:**
  - Usage → "Otto usage" tab:
    - `{:catch e}<LoadState what="Otto usage" error={e instanceof Error ? e.message : …} empty onretry={() => (view = 'overview')} />`.
    - LoadState renders that callback as a button labelled **Retry** (`LoadState.svelte:67`), but clicking it switches to the Overview tab and doesn't retry.
    - The error text is the browser's raw chunk error ("Failed to fetch dynamically imported module: …").
  - Split view Database pane:
    - `{:catch err}<div role="alert">Couldn’t load the Database pane: {err.message}</div>`.
    - It shows the raw message, with no Retry and no Reload.
  - The shell already defines the right pattern for the same failure: LoadState with a plain-language cause ("Otto may have been updated in the background…"), a real Retry, and an `errorActions` "Reload Otto".
- **Fix:** In both places use the shell's wording, a Retry that re-invokes the import (a fresh `import()` promise held in state), and `errorActions` → "Reload Otto". In Usage, if going back is wanted, add it as a separate, honestly labelled "Back to overview" action.

### ux-design-3-06 — minor — Hand-built "Xm ago" ladders remain on the Git graph, Product tabs and session pane

- **Files:**
  - `ui/src/modules/git/GraphView.svelte:2218-2227` (`fmtDate`, used for every commit row `:3467` and the commit detail `:3595`)
  - `ui/src/modules/product/OverviewTab.svelte:1244-1258`, `DiscoveryTab.svelte:139-153`, `ChatTab.svelte:119-133`, `RefineTab.svelte:152-166`. These are four copies of the same `relDate`, used at OverviewTab `:1606`, `:1749`, `:1798`, ChatTab `:174`, RefineTab `:196`, `:238` and DiscoveryTab `:214`.
  - `ui/src/lib/status.ts:340-346` (`agoLabel`, which feeds SessionView's "Suspended 3h ago" note)
  - `ui/src/modules/product/design/DesignArena.svelte:242` ("saved 12s ago")
- **Evidence:**
  - Each is a local seconds/minutes/hours/days ladder computed from `Date.now()`, so it doesn't tick (GraphView and the product tabs). Each uses its own vocabulary: "just now" vs `rel()`'s "now"/"12s ago", a 30-day cutoff, and `toLocaleDateString` formats that differ per file.
  - `content.md` §7: "Use the shared helpers. Don't write another local `timeAgo`". Use `rel(ts)` from `lib/stores/now.svelte.ts`, which ticks reactively.
  - Iteration 2 fixed the six surfaces it listed. These are the remaining copies, on core paths (the Git graph and every Product story tab).
- **Fix:** Replace each with `rel(iso)`, or `rel(iso, { compact: true })` on dense rows, and add `title={new Date(iso).toLocaleString()}` per §7 ("relative in lists, absolute on hover"). Have `suspendedNote` take the label from `rel()`. Then delete the five local functions.

### ux-design-3-07 — nit — "Could not …" voice and a second typed-mismatch wording

- **Files and evidence:**
  - The app voice is "Couldn’t …" (`content.md:84-93`; used by hundreds of call sites). These still say "Could not":
    - Rooms core: `ui/src/modules/rooms/RoomPage.svelte:79`, `:125`, `:130`; `RoomLobby.svelte:58`; `RecapPanel.svelte:84`, `:85`, `:87`; `RoomRecap.svelte:37-38`; `RoomSettingsModal.svelte:24`; `StartRoomModal.svelte:24`
    - `ui/src/modules/settings/Login.svelte:21`, `:28` ("Could not sign in — daemon unreachable?")
    - `ui/src/modules/git/ProofPackView.svelte:74` ("Could not copy")
  - `ui/src/modules/kubernetes/actions.ts:93-96` (`typedConfirm`, the agent-driven typed confirm) toasts "Name did not match — Nothing was changed." Every other typed gate (`confirmProd.ts:66-68`, the database gate, ImportDialog) says "Nothing was changed — What you typed didn’t match, so the action was canceled."
- **Fix:** Change these strings to "Couldn’t …". In `typedConfirm`, use the shared mismatch wording, or call `confirmProd({ typed })` directly.
