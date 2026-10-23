# Quality iteration 6 (final): ux-design lens

Reviewer: ux-design (final pass). Static and read-only: no cargo, npm, builds, tests or e2e; grep/rg/sed/git only.
Base: `fix/quality-iter6-20261023` @ `2065f00e9` (clean tree apart from untracked wave-3 review files, so the code is the code wave 3 reviewed).
Scope: `ui/src` (the UI is the whole of this lens), plus `docs/design/guidelines/` and the AGENTS.md design rules as the yardstick. Excluded: `target/`, `node_modules/`, `docs/reviews/`.
The branch diff against `origin/main` is 864 files (+74,339 / −37,559); 423 of them under `ui/src`.

## Score

Rubric: 10.0 minus 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit, per confirmed open finding.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 3 | 0.30 |
| nit | 2 | 0.06 |
| **Total** | 5 | **0.36** |

**Score: 10.0 − 0.30 − 0.06 = 9.64 / 10.**

Earlier scores: iteration 5 8.55; iteration 6 wave 0 9.31, wave 1 9.81, wave 2 9.97, wave 3 9.97.

Why this is lower than wave 3: wave 3's one finding (6w3-01) is still open, because no commit landed after wave 3. The other four findings are new. They come from this final pass widening the area list to code this iteration moved but no wave had read with this lens:
- the git graph split (`modules/git/graph/*`)
- the database store split (`lib/stores/database/*`)

All four problems already existed on `origin/main` and were moved verbatim. The fixes did not introduce them. They are real, confirmed defects, so they count.

## Previous-finding status (wave 3)

| Id | Status | Evidence |
|---|---|---|
| ux-design-6w3-01 (nit) | **Still open** → carried as 6f-01 | `RecapImage.svelte:12` still sets `error = loadErrorText(e)`. `recap-client.ts:21` still throws `'Couldn’t load the recap file. Try again.'`. The compact `LoadState` at `:15` therefore still reads "Couldn’t load this screen sample. Couldn’t load the recap file. Try again." `RoomRecap.svelte:111` still uses `what="the local recap engines"`, while `:54` still says "Couldn’t check the local recap engines.". |

Totals: 0 fixed, 0 partly fixed, 1 still open, 0 invalid.

**Regression check on every wave-0 finding (6-01 … 6-09), all still fixed:**

| Finding | What still holds |
|---|---|
| 6-01 | `ResultsGrid.svelte:7-9` uses `lazyComponent`. The only bare `{#await import(` left is `UsagePage.svelte:711`, which has pending, then and catch, as accepted before. |
| 6-02 | A scripted scan of every `<EmptyState …>` finds no error-titled instance without `tone="error"`. `StructureView.svelte:660` is a `LoadState`. |
| 6-03 | `AnalysisTab` has `detailLoading` and `detailError`. |
| 6-04 | `AgentPage.svelte:442-444` shows `agentsError` through `LoadState`. `MockupAnnotations.svelte:406` uses `LoadState`, and its empty line no longer shows on error. `LinkedCanvases` has `loadError`. `RecoveryView.svelte:137` gates its empty line on `!error`. |
| 6-05 | `sessionAttach.ts` is in place. |
| 6-07 | `COPY_FAILED_BODY` is in `lib/clipboard.ts`. No direct `navigator.clipboard.writeText` is left in module code. |
| 6-08 | No user-visible "Could not" remains in modules. The hits left are agent-facing `UiCommandError`s or non-UI comments. |
| 6-09 | `OpenApiView.svelte:208-210` shows an `EmptyState` when the spec has no operations. |

## Areas covered

- **Re-verification:** 6w3-01, plus the regression check of 6-01 … 6-09 above.
- **The wave-0 area list, re-walked against current code:**
  - Product: AnalysisTab, PlanTab, RewriteTab, OverviewTab, DraftEditor, StoryTranscripts, and the JiraIssueSection states (Jira details and development-info `LoadState`; writes stay confirmed through `JiraIssueEditor`).
  - Product: MockupAnnotations, LinkedCanvases, SourceSearch, TestCasesTab and `caseOrder`.
  - Design Hall Site studio: Inspector, LeftPanel, assist (OttoPanel, VariantsTray), and the new `LinkListField`.
  - Brokers TopicDetail, GroupsTab and ReplayPanel (unchanged).
  - Personal Agents (AgentPage) and Mission Control WorkItemDetail.
  - Vault: RefineDrawer, RecoveryView, OpenApiView, VaultPage.
  - Rooms: RoomRecap, RecapImage, RecapSettingsModal, GameMatch, the Kart and Shooter covers, and the new shooter `hud/*`.
  - MCP OttoServerHome; JiraIssuePicker; FileTree.
- **Code this iteration changed (`git diff origin/main`), read with this lens:**
  - The git graph split: `graphActions`, `graphMenus`, `RefSidebar`, `RefSideLists`, `CommitDetailPane`, `RefPopover`.
  - Workflows split: RunMonitor, RunApprovals, RunInputPanel, RunFinalOutput, RunContextSidebar. These are verbatim moves of the `WorkflowsPage` blocks. The approval banner uses `ApprovalActions`/`ApprovalOutcome`, and the results-destination line sits above Run.
  - Database store split: connections, queryRuns, objectOps, schemaTree. All of their toasts were read.
  - Home classrooms (`load.svelte.ts`, DoorBubbles, ClassroomsBox states).
  - Kubernetes: `lib/k8s/actions` (typed confirms get named verbs from the UI-command caller) and MonitorSettings/MonitorCluster.
  - AWS: S3TransferBars, LogsView, MetricsPanel.
  - Swarm: AgentEditor identity read-only note, RunsList's disabled "open session" with a reason, `access.ts`.
  - AdminSessions paging.
  - Insights `scheduleStatus`, Proof `assembleOutcome`, Scheduled tasks `firePreview`.
  - Browser live view, LiveEngineSetup, Studio3D assist outcome, SessionMcpProfiles skipped-server notice, CapsBanner.
  - Settings: Users, SelfImprovement, TrustSafety and the rest of the a11y sweep. Assistant MemoryTab, Loops, Canvas ConversationPanel, SnipEditor.
- **Cross-cutting greps over all of `ui/src`, and over the added lines of the branch diff:**
  - Hex colours, native `confirm`/`alert`/`prompt`, raw `e.message` as a headline, and `confirmLabel` "OK"/"Yes"/"Confirm". No new hits. The k8s `typedConfirm` default "Confirm" is never reached: the one caller passes the verb.
  - New `.primary` buttons. Each is the single primary of its surface.
  - New `role="dialog"`/menus. Only `RefPopover`, a verbatim move.
  - `EmptyState` used as an error, bare lazy awaits, "Could not", and clipboard paths with no next step.
  - All 35 `toast*('Couldn’t load/list/fetch/read …')` sites. Every one is action-triggered or keeps an inline state, except the object search below (6f-04).
  - Load functions whose catch sets no inline error, in swarm, loops, assistant, snip, proof, canvas, workflows, settings, mcp and desktop. All of them set an inline error, except action-triggered loads such as Proof "Load content".
  - "Couldn’t <non-verb>" wording, which found 6f-04's toast.

## Areas NOT covered

- Screenshots of any kind: game HUD at phone width, phone and tablet layouts, light and dark. This review is static.
- Deep flow reads of the Swarm Kanban and OrgTree interactions, Assistant threads and tasks, the Canvas editors, and the Workflows node inspector. These were reached by greps and diffs only.
- The tray pages (`modules/desktop`) beyond the BarHost load path.
- MCP ProfilesTab, AuditTab, ExposePanel and TokensPanel. Only their load and error paths were read.

---

## Findings

### ux-design-6f-01: nit. Rooms recap: the `LoadState` wrappers double or change the error copy (6w3-01, still open)

- **Files:**
  - `ui/src/modules/rooms/RecapImage.svelte:12`, `:15`
  - `ui/src/modules/rooms/recap-client.ts:13`, `:21`
  - `ui/src/modules/rooms/RoomRecap.svelte:54`, `:106`, `:111`
- **Evidence:**
  - `recapBlob` throws `new Error('Couldn’t load the recap file. Try again.')` on any non-OK response (`recap-client.ts:21`).
  - `loadErrorText` passes an `Error`'s message through unchanged (`lib/loadError.ts:28`).
  - `RecapImage` renders that message as the detail of `<LoadState what="this screen sample" … {error} …>`. On any HTTP failure the line reads "Couldn’t load this screen sample. Couldn’t load the recap file. Try again." [Retry]: two headlines, plus "Try again." beside a Retry button.
  - `RoomRecap.svelte:111` shows the failed capability probe as "Couldn’t load the local recap engines.". The same failure reads "Couldn’t check the local recap engines." in the banner (`:54`, `:106`).
- **Fix:**
  - Make `recap-client.ts:21` (and the `:13` fallback) throw a cause, not a headline. For example ``throw Object.assign(new Error(`The daemon answered ${response.status}.`), {status: response.status})``, so `loadErrorText` maps 401, 403 and 404 to its standard lines. Drop "Try again.".
  - Use `what="the recap engine status"` at `RoomRecap.svelte:111`, and the same noun at `:54`.

### ux-design-6f-02: minor. Git graph: deleting a remote branch, and the create-tag push prompt, confirm without "who sees it", unlike their sibling remote-tag actions

- **File:** `ui/src/modules/git/graph/graphActions.svelte.ts`
  - `:173-178` `deleteLocalBranch(…, alsoRemote=true)`: "Delete branch “x” locally AND “origin/x” on the remote …"
  - `:215-221` `deleteRemoteBranch`: "Delete branch “origin/x” on the remote “origin”? This cannot be undone."
  - `:137-141` `createTagAt`: "Push tag “v1” to origin?"
- **Evidence:**
  - All three go to a shared remote, so other people see them. Yet all three use a bare `confirmer.ask`, with no visibility line.
  - The same file's `pushTag` (`:233-238`) and remote `deleteTag` (`:244-246`) use `confirmOutward` with a `who`: "Everyone with access to the remote loses the tag; CI or release jobs may react." That was the 5-15 fix.
  - `patterns.md` §5 lists "pushing" as outward. It requires where, what and **who** for every outward confirm, and exempts only a plain push of the current branch.
  - Deleting a remote branch can close the open PRs whose head it is, and can fire CI. The person is never told.
- **Fix:**
  - Route the remote half through `confirmOutward`. For example, `{ verb: 'Delete branch', where: `${remote} · branch ${name}`, danger: true, who: 'Everyone with access to the remote loses the branch; open PRs from it close and CI may react.' }`. For the local + remote variant, keep the "locally AND" wording in `what`.
  - Give the create-tag push step the same `who` as `pushTag` (see 6f-03 for its buttons).

### ux-design-6f-03: minor. Create tag: "Cancel" on the "Push tag to origin?" step still creates the tag

- **File:** `ui/src/modules/git/graph/graphActions.svelte.ts:136-147`
- **Evidence:**
  - After the name (and message) prompts, `createTagAt` asks `confirmer.ask('Push tag “${name}” to origin?', { title: 'Push tag', confirmLabel: 'Push' })`. The other button is the default "Cancel".
  - The result only sets `push`. `await mutate('/tag', { name, sha, push })` runs whatever was answered. So "Cancel", Escape or closing the dialog all create a local tag.
  - `content.md` §2 defines Cancel as "dismisses a dialog, or aborts a pending operation that hasn't started". Here it silently commits the operation. Someone backing out of tag creation at the last step gets a tag they then have to find and delete.
  - The helper supports a named second choice: `cancelLabel` (`lib/confirm.svelte.ts:54`), already used for "Keep editing" and "Keep running".
- **Fix:**
  - Ask a three-way question with `confirmer.choose`: "Create and push" (primary), "Create locally" and Cancel. Cancel then aborts.
  - Or, at minimum, pass `cancelLabel: 'Create without pushing'`, so the button says what it does.

### ux-design-6f-04: minor. Database object search: a failed search reads "No object matching …", with an ungrammatical "Couldn’t object search" toast

- **Files:**
  - `ui/src/lib/stores/database/schemaTree.svelte.ts:211-215`
  - `ui/src/modules/database/SchemaTree.svelte:576-586`
- **Evidence:**
  - On a failed `/search-objects` call, the store sets `this.db.objectSearchHits = []` and calls `toastError('Couldn’t object search', e)`.
  - The tree then branches on `hits.length === 0` and renders `No object matching "{query}" in this schema — try All schemas.`. No error state exists: `objectSearchError` doesn't exist anywhere. After the toast fades, the failure reads as a real "no such table" answer and sends the person to widen a search that never ran.
  - Rules broken:
    - `components.md`: "A failed load can therefore never read as 'No X yet'".
    - `content.md` §4: an inline error for failed loads. The title starts with the verb the user tried, but "object search" is not a verb.
  - The empty line also uses straight quotes, where `content.md` §3 asks for curly quotes in prose.
- **Fix:**
  - Add `objectSearchError` to the store. Set it, with `loadErrorText(e)`, instead of `hits = []` and the toast.
  - In `SchemaTree.svelte`, render `<LoadState what="matching objects" variant="compact" error={database.objectSearchError} empty onretry={() => database.searchObjects(database.objectSearchQuery, database.activeDb ?? undefined)} />` ahead of the "No object matching" branch.
  - If a toast stays, word it "Couldn’t search objects". Use `“…”` in the empty line.

### ux-design-6f-05: nit. Git graph menus: "Checkout" and "Check out" side by side, and "Push tag to origin" lacks the ellipsis of a confirming action

- **File:** `ui/src/modules/git/graph/graphMenus.ts`
  - `:184` (remote branch) and `:252` (local branch) use `label: 'Checkout'`.
  - `:105` ("Check out {name}"), `:117` ("Check out commit (detached)") and `:359` ("Check out tag (detached)") use the verb form.
  - `:364` `label: 'Push tag to origin'`.
- **Evidence:**
  - `content.md` §2: buttons and menu items are verbs, with "one word per concept everywhere". "Checkout" is the noun. The same menus use the verb "Check out" three times.
  - `content.md` §2: "A trailing ellipsis … means the action asks for more input before it happens ('Delete…' in a menu that then confirms)." `pushTag` always opens `confirmOutward` (`graphActions.svelte.ts:234`), and every other confirming entry in this file ends with "…" ("Delete tag…", "Revert commit…", "Drop stash…").
- **Fix:** Use `'Check out'` at `:184` and `:252`, and `'Push tag to origin…'` at `:364`.
