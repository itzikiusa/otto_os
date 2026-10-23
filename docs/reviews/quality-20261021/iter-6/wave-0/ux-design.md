# Quality iteration 6 (wave 0): ux-design lens

Reviewer: ux-design. This review is static and read-only. I ran no builds, tests or e2e.
Base: `fix/quality-iter6-20261023` @ `02a64019b`.
Scope: all of `ui/src`, including the room games (`modules/rooms/games/**`), room recaps and the classroom. Excluded: `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: start at 10.0. For each open finding, subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 6 | 0.60 |
| nit | 3 | 0.09 |
| **Total** | 9 | **0.69** |

**Score: 10.0 − 0.60 − 0.09 = 9.31 / 10.** Iteration 5 scored 8.55.

Thirteen of the 16 iteration-5 findings are fixed. The other three were fixed only at the sites the report listed. The same class of problem is still open elsewhere, so each comes back below as a narrower finding:

- 5-01 lives on as 6-01 (minor). The shell and Product parts are fixed. What is left is the Database results-grid dialogs.
- 5-02 lives on as 6-02.
- 5-12 lives on as 6-07.

The other new findings come from areas no earlier pass covered: Product AnalysisTab, Personal Agents detail, Product annotations and canvases, the MCP server home, room recaps and the games' copy.

## Previous-finding status (iteration 5)

| Id | Status | Evidence |
|---|---|---|
| ux-design-5-01 (major) | **Partly fixed → 6-01** | The shell overlays now use `lazyComponent` + `<LazyMount quiet onerror>` (`shell/overlays.svelte.ts:3`). So do the Product tabs; no bare `{#await import(` is left in `shell/` or `ProductPage.svelte`. `ResultsGrid.svelte:1798-1857` still has 7 bare awaits, including the write ReviewModal. |
| ux-design-5-02 | **Listed sites fixed; class open → 6-02** | None of the 18 listed sites is still an `EmptyState` "Couldn’t …". Eight other sites still are (see 6-02). |
| ux-design-5-03 | Fixed | `DiagramView.svelte:45` has `graphError`. `:453-454` renders `LoadState` with Retry before "No schema selected". |
| ux-design-5-04 | Fixed | FIXES.md: SessionsBox uses `LoadState` with "—" stats. |
| ux-design-5-05 | Fixed | `sendTo.ts:61-70` checks `database.connectionsError` before the "No database connections" toast. |
| ux-design-5-06 | Fixed | `ApprovalsTab.svelte:35`, `:94-95`: `userLabel()` resolves to "you", then a member name, then a short id. The raw id moves into `title`. |
| ux-design-5-07 | Fixed | `WidgetCard.svelte:176-178`: `LoadState` owns loading, error and stale. |
| ux-design-5-08 | Fixed | `Dashboards.svelte:105` `addBlocked`, `:241-245` "Runs on …", `:286-290` "Columns come from this widget’s last run.", `:297` Add disabled without a connection. |
| ux-design-5-09 | Fixed | `MonitorSettings.svelte:58-60` adds `aria-invalid` and `aria-describedby`, `:172` focuses the first invalid field, and `:335` shows the summary line. There is no toast. |
| ux-design-5-10 | Fixed | `SchoolList.svelte:85-87`: a detention row says "Show" and calls `rowClick`. Release is an explicit button, disabled for people who can't manage the session (`:96-99`). |
| ux-design-5-11 | Fixed | No "Could not" and no `LoadState` remain in `GamesHub.svelte` or `GameGuest.svelte`. |
| ux-design-5-12 (nit) | **Listed sites fixed; class open → 6-07** | All 8 listed sites now say "Select the text and copy it manually." About 25 other copy paths still don't. |
| ux-design-5-13 | Fixed | `PublishModal.svelte:57`, `:91`, `:104`, `:121`: `{ what, detail }`, with "Couldn’t export the site." and similar headlines. |
| ux-design-5-14 | Fixed | `ProfilesTab.svelte:120-124`: every menu row gets `title: admin`. `:188` Make default has `title={… NEEDS_ADMIN}`. |
| ux-design-5-15 | Fixed | `GraphView.svelte:1243`: the revert row no longer has `danger`. `:1629-1631`: deleting a remote tag goes through `confirmOutward` with a `who`. |
| ux-design-5-16 | Fixed | `ResourceTable.svelte:34`, `:118`: `onclearfilter`. The title reads "No {kind} match “{filter}”" and the action is "Clear filter". |

Totals: 13 fixed, 3 partly fixed (carried as 6-01, 6-02, 6-07), 0 invalid.

## Areas covered this iteration

- All 16 iteration-5 findings, re-checked against the current code (table above).
- **Product (open since iteration 5):** AnalysisTab past-run load (`selectHistory`), PlanTab (`loadError` → `LoadState`, fine), RewriteTab (`LoadState` + stale bar, fine), OverviewTab version picker and transcripts, MockupAnnotations, LinkedCanvases, SourceSearch filters.
- **Design Hall Site studio internals (open since iteration 5):**
  - SiteStudio: library load, insert, page and section confirms, layer rename prompt.
  - LeftPanel: library error with Retry.
  - Assist: OttoPanel load and `LoadState`, apply-variant and apply-conflict confirms, VariantsTray Apply titles, TurnMessage "Apply on top…". This passes `patterns.md` §2 and its documented Design Hall exception.
- **Brokers (open since iteration 5):**
  - TopicDetail: consume, peek and tail states, produce validation, the prod confirm with key, size and partition, config change confirm, typed-name delete.
  - GroupsTab: opens on the first group, plus a 403 ACL banner.
  - ReplayPanel.
- **Personal Agents and Mission Control detail (open since iteration 5):** AgentPage (header primaries, agent-not-found path, schedules and runs `LoadState`), AgentActivity, AgentAutonomy, AgentMemoryInspector, AgentDocuments, WorkItemDetail (load, error, Retry, copy ID).
- **Vault (open since iteration 5):**
  - RefineDrawer: in-place edit + Undo refine, which is the documented exception in `patterns.md` §"outward".
  - VaultPage: note-open error banner with Retry and Dismiss.
  - RecoveryView (trash and history), OpenApiView.
- **Rooms:**
  - RoomRecap: consent and capture controls, prepare modal.
  - RecapPanel and RecapSettingsModal.
  - GameMatch: invite copy, error and reconnect.
  - KartGame and ShooterGame: boot, error and loading covers.
- **MCP:** OttoServerHome (session attach toggle, gateway tools).
- **Agents:** JiraIssuePicker states, plus panels FileTree root and viewer errors.
- **Cross-cutting greps across `ui/src`:**
  - Bare `{#await import(`.
  - `EmptyState` used as an error without `tone="error"` (scripted scan of every `<EmptyState …/>`).
  - `Couldn’t copy` / `copyText` paths with no next step.
  - "could not / cannot / failed to" in user strings.
  - `toastError('Couldn’t load …')`, tracing what each view shows after the toast.
  - Hand-rolled "Couldn’t load" blocks and whether each has a Retry.
  - Module `*Page.svelte` files without `PageHeader`/`PageBody`. The misses are embedded sub-pages or the tray, or were accepted before.
  - `PageHeader` blocks with more than one `.primary`. All three hits are mutually exclusive branches.
  - Hand-rolled `role="dialog"`. All hits are popovers, the palette, a lightbox or the tray.
  - Toasts used for field validation. The hits guard already-disabled buttons.

## Areas NOT covered yet (next iteration)

- Game in-match HUD copy and the kart/shooter menus at phone width. That needs screenshots; this review is static.
- Swarm (Kanban, OrgTree, GoalsPanel) and Loops detail flows. Only greps reached them.
- Assistant module threads and tasks, the Snip editor, Proof page artifact flows beyond the media error, and the desktop tray pages.
- Canvas module (CanvasPage, agent assist) and the Workflows inspector after its iteration-2 split.
- Settings pages other than IssueAccounts and Channels (BackupRestore, PluginsSettings, SkillsLibrary install flow).
- Phone and tablet layouts of every module. No screenshots were taken.

---

## Findings

### ux-design-6-01: minor. Database results grid: 7 lazy dialogs are still bare `{#await import()}` blocks, so a failed chunk makes "Review" or "Export" a silent, sticky no-op (the rest of 5-01)

- **File:** `ui/src/modules/database/ResultsGrid.svelte`:
  - `:1798` DocEditor
  - `:1802` RawDocViewer
  - `:1806` ExportDialog
  - `:1810` RecordDiff
  - `:1822` AggregateBuilder
  - `:1835` ReviewModal, the write review
  - `:1849` ContextPacketDialog
- **Evidence:**
  - Each site is `{#await import('./X.svelte') then m}<m.default …/>{/await}`. None has a pending branch or a `{:catch}`.
  - `flow.reviewSql` (`:1834`) stays set when the import fails. The person clicks to review a write and no review appears. Later clicks set an already-set flag, so the review stays dead until reload. The same holds for `showExportDialog` (`:1805`), `compare` (`:1809`), `pipelineMounted` (`:1821`) and `sendToAgentOpen` (`:1848`).
  - The fix pattern already exists and was applied to the shell in iteration 5: `shell/overlays.svelte.ts:3` ("A bare `{#await import()}` showed nothing while the…") and `<LazyMount quiet onerror>`.
  - `UsagePage.svelte:708-714` shows the other accepted form: pending, then, and `:catch` with Retry.
- **Fix:**
  - Hoist each import into a module-level `lazyComponent(() => import('./ReviewModal.svelte'))` (and the same for the other six).
  - Render each with `<LazyMount lazy={…} what="the write review" quiet props={{…}} onerror={() => flow.closeReview()} />`.
  - In each `onerror`, reset the flag that opened the dialog (`showExportDialog = false`, `compare = null`, `pipelineOpen = false`, `sendToAgentOpen = false`, `flow.closeDocEditor()` and so on), so the next click retries.

### ux-design-6-02: minor. `EmptyState` is still used as a load-error block at 8 sites: no danger tile, no `role="alert"`, and not `LoadState`

- **Files:**
  - `ui/src/modules/database/StructureView.svelte:658-664`: "Couldn’t load object", with `icon="x"`.
  - `ui/src/modules/aws/LogsView.svelte:543-550` ("Couldn’t list log groups"), `:625-632` ("Couldn’t read log events") and `:720-727` ("The Insights query failed").
  - `ui/src/modules/aws/MetricsPanel.svelte:144-150` ("CloudWatch access denied") and `:152-158` ("Couldn’t load metrics").
  - `ui/src/modules/mission-control/MissionControlPage.svelte:383-390`: "Couldn’t load Mission Control", with `icon="radar"`.
  - `ui/src/modules/git/PrList.svelte:194-201`: `title={errorTitle}`, default "Provider unreachable".
- **Evidence:**
  - I scanned every `<EmptyState …/>` with a script. These 8 carry a "Couldn’t …" title or a Retry action but no `tone="error"`.
  - `EmptyState` adds the danger icon tile and `role="alert"` only for `tone="error"`, as 5-02 established. So these failures render as neutral empty states and are not announced. Three of them use a non-warning icon (`x`, `chart`, `radar`).
  - Iteration 5 converted the 18 listed AWS and K8s siblings (EksView, RdsView, AthenaView, Ec2View, S3Browser, SqsView) to `LoadState`. These are the same files' neighbours.
  - `components.md`: "use them, don't hand-roll an error block. A failed load inside an `{#if}` chain that already handles loading and empty is still `LoadState`".
- **Fix:**
  - Replace each with `<LoadState what="…" error={…} empty onretry={…} />`.
  - Keep the AWS "Sign in" branch through LoadState's `errorActions` snippet, as the converted AWS siblings do.
  - PrList: keep `errorTitle` as the `what`, or pass `tone="error"` if it must stay an EmptyState.

### ux-design-6-03: minor. Product analysis: a past run that fails to load reads as "Configure lenses above and click Run analysis.", and re-picking it does nothing

- **File:** `ui/src/modules/product/AnalysisTab.svelte:299-316`, `:510-519`, `:551`, `:865-869`
- **Evidence:**
  - `selectHistory(a)` sets `activeId = a.id; activeDetail = null;`. On a throw it only calls `toastError('Couldn’t load analysis', e)` (`:314`).
  - The run area branches on `{#if activeDetail}` (`:551`), then `{:else if !running}<div class="muted">Configure lenses above and click Run analysis.</div>` (`:865-866`). A failed load therefore shows the "start a new run" invitation once the toast fades.
  - While the detail is still loading, the same invitation shows: there is no loading branch.
  - Retrying is impossible. The first line of `selectHistory` is `if (a.id === activeId) return;` (`:300`), and the history `<select>` already has `value={activeId}` (`:518`), so choosing the same run fires no change. The person has to pick a different run and then come back.
  - The sibling history list in the same file does this right: `historyError` shows "Couldn’t load past runs" with Retry (`:529-531`).
  - `components.md:504`: a failed load must never read as "No X yet". `content.md` §4: failed loads are inline, not a toast.
- **Fix:**
  - Add `detailLoading` and `detailError` state.
  - Render `<LoadState what="this analysis run" loading={detailLoading} error={detailError} empty={!activeDetail} onretry={() => retrySelect()} />` ahead of the `!running` branch.
  - In `selectHistory`, allow a re-select when `activeDetail === null` (`if (a.id === activeId && activeDetail) return;`), and drop the toast.

### ux-design-6-04: minor. Four views turn a failed load into "not found" or "nothing yet" (Personal agent, mockup annotations, canvas picker, vault edit history)

- **Files and evidence:**
  - **Personal agent page:** `ui/src/modules/personal-agents/AgentPage.svelte:446-459`, `:97-104`.
    - When the agent isn't in the store, the page shows a Skeleton while `personalAgents.loadingAgents`. Otherwise it shows `EmptyState title="Agent not found" body="It may have been deleted, or it belongs to another workspace."`.
    - The store records a failure in `personalAgents.agentsError` (`lib/stores/personalAgents.svelte.ts:29`, `:91`), but AgentPage never reads it. The list page does (`PersonalAgentsPage.svelte:78`, `:190-194`).
    - The lookup effect sets `lookedUp = key` before loading, so nothing retries. A deep link or reload on a flaky daemon tells the person their agent was deleted.
  - **Mockup annotations:** `ui/src/modules/product/MockupAnnotations.svelte:72-80`, `:395-399`. `loadNotes` only toasts "Couldn’t load annotations". The panel then shows "No annotations yet. Click the mockup to drop a pin.", which invites the person to re-add pins that already exist.
  - **Linked canvases picker:** `ui/src/modules/product/LinkedCanvases.svelte:50-55`, `:154-157`.
    - `openPicker` toasts on a failed `canvas.loadScenes()`. The picker then shows "No other canvases to link.".
    - That same line also shows while the load is still running, because there is no loading branch.
  - **Vault edit history:** `ui/src/modules/vault/RecoveryView.svelte:118`, `:137`.
    - On an error, the history mode shows the "Couldn’t load the edit history" alert and also "No recorded edits. History begins with…". The empty line is gated only on `visibleRevisions.length === 0`.
    - The trash branch in the same file guards correctly with `&& !error` (`:123`).
- **Rule:** `components.md:504` ("A failed load can therefore never read as 'No X yet'"). AGENTS.md ("A failed load shows inline with Retry, not as a raw exception in a toast").
- **Fix:**
  - AgentPage: add `{:else if personalAgents.agentsError}<LoadState what="this agent" variant="page" error={personalAgents.agentsError} empty onretry={() => ws.currentId && personalAgents.loadAgents(ws.currentId)} />` before "Agent not found". Clear `lookedUp` on failure.
  - MockupAnnotations and LinkedCanvases: keep a `loadError`. Render `<LoadState variant="compact" what="annotations" / "your canvases" …>` with Retry, and show a Skeleton while the picker loads.
  - RecoveryView: gate the history empty line on `!error`, as the trash branch does.

### ux-design-6-05: minor. MCP server home: a failed load of the session-attach setting shows a guessed "attached" checkbox, and neither it nor the gateway-tools load offers Retry

- **File:** `ui/src/modules/mcp/OttoServerHome.svelte:237-247`, `:333-352`, `:273-286`, `:512-515`
- **Evidence:**
  - `loadAttach` sets `attach = null` and, on a throw, `attachError = e.message` (raw).
  - The checkbox renders `checked={attach?.attached ?? true}` and `disabled={!attach || …}` (`:337-338`). After a failure, the person sees a disabled, ticked "Attach to sessions in {workspace}" box: a state the daemon never reported.
  - Below the box is `<p class="warn">Couldn’t load session attachment: {attachError}</p>` (`:352`). It has no Retry and no `role="alert"`. The only recovery is switching workspaces.
  - A failed gateway-tools load renders `<p class="muted small">Couldn’t load gateway tools: {gatewayError}</p>` (`:514-515`). It is muted, has no Retry, and its raw `e.message` is set at `:282`.
  - `components.md`, "Error" and "use them, don't hand-roll an error block". `content.md` §4: the title is what failed, and raw detail goes last.
- **Fix:**
  - While `attachError` is set, don't render the checkbox. Render `<LoadState what="the session-attach setting" variant="compact" error={attachError} empty onretry={() => wsId && loadAttach(wsId)} />` in its place.
  - Use the same `LoadState` for the gateway tools, with `onretry={() => loadGatewayTools(wsId)}`.
  - Use `loadErrorText(e)` for both messages.

### ux-design-6-06: minor. Room recap prepare dialog: a failed "Ask everyone to consent" shows the raw server message and a "Retry checks" button that retries the wrong step

- **File:** `ui/src/modules/rooms/RoomRecap.svelte:37-38`, `:85`, `:92`
- **Evidence:**
  - `create()` catches with `error = e instanceof Error ? e.message : 'Couldn’t prepare the recap.'`. `openPrepare()` does the same with `'Couldn’t check local engines.'`. The "Couldn’t …" headline appears only when the thrown value is not an Error.
  - `recapRequest` throws `new Error(problem.message …)` (`recap-client.ts:12`), and a timeout surfaces the raw AbortError text. So the person reads just the server's message, with no "Couldn’t prepare the recap." headline.
  - Inside the modal, `{#if error}<p role="alert">{error}</p><button class="btn" onclick={openPrepare}>Retry checks</button>{/if}` (`:92`) appears for both failures. After a failed create, "Retry checks" re-runs the capability probe, not the create. That violates `content.md` §4 ("Name the real control").
  - The same `error` also renders in the banner behind the modal (`:85`), so the alert is announced twice.
- **Fix:**
  - Keep `{ what, detail }`, as PublishModal now does: "Couldn’t check the local recap engines." or "Couldn’t prepare the recap.", plus `loadErrorText(e)` dimmed.
  - Show "Retry checks" only for a failed capability check. For a failed create, the primary "Ask everyone to consent" stays the retry.
  - Render the banner's `{#if error}` only when `!prepare`.

### ux-design-6-07: nit. About 25 clipboard-failure paths still don't say what to do next, one is silent, and the game invite bypasses the shared clipboard helper

- **Files:**
  - `toastError('Couldn’t copy…', e)`, where the body is the thrown `copyTextOrThrow` message "Clipboard unavailable — the browser blocked the copy":
    - `ui/src/modules/aws/`: `SqsView.svelte:331`, `S3Browser.svelte:396`, `RdsView.svelte:133`, `AthenaView.svelte:415`, `Ec2View.svelte:183`, `EksView.svelte:127`, `LogsView.svelte:342`
    - `ui/src/modules/snip/SnipEditor.svelte:284`
    - `ui/src/modules/agents/conversation/`: `ToolStep.svelte:219`, `PreviewPanel.svelte:80`, `TurnItem.svelte:81`, `:89`
    - `ui/src/modules/git/GraphView.svelte:942`
    - `ui/src/modules/product/InjectTab.svelte:62`
  - A body that only restates the cause:
    - `ui/src/modules/api/`: `RequestBuilder.svelte:684`, `ResponseViewer.svelte:256`, `JsonTree.svelte:45`
    - `ui/src/modules/agents/ShareModal.svelte:179`
    - `ui/src/modules/design-hall/brand/edit.ts:15`
    - `ui/src/modules/agents/SessionView.svelte:594`
    - `ui/src/modules/git/RepoView.svelte:225`
    - `ui/src/modules/brokers/TopicDetail.svelte:653`
    - `ui/src/modules/usage/AttributionDrilldown.svelte:110`
  - Silent on failure: `ui/src/modules/mission-control/WorkItemDetail.svelte:317` (`if (await copyText(v)) toasts.success('Copied ID')` has no else).
  - `ui/src/modules/rooms/games/GameMatch.svelte:128` calls `navigator.clipboard.writeText` directly.
- **Evidence:**
  - `content.md:100`: "Clipboard failures say what to do next ('Select the text and copy it manually.'), not the title again." Iteration 5 fixed the 8 listed database, swarm and scheduled-task sites. The rest of the UI uses the same pattern.
  - GameMatch skips `lib/clipboard.ts`. That module's header explains that `navigator.clipboard` is missing on the LAN and self-signed origins where guests reach a game. The game invite is exactly the copy most likely to run there. Its fallback text, "Copy the invitation from the field below.", also points the wrong way: the field sits beside the button in the same row (`:168`).
- **Fix:**
  - Change the thrown message in `lib/clipboard.ts:59` (`copyTextOrThrow`) to "The clipboard isn’t available here. Select the text and copy it manually." That one line fixes every `toastError` site.
  - Append the same sentence to the "isn’t available" bodies, or route them through `copyWithToast`.
  - Add the missing `else` in WorkItemDetail.
  - Use `copyText()` in GameMatch, and say "Select the invitation link and copy it."

### ux-design-6-08: nit. "Could not", "Cannot open" and "failed to" still appear in user-visible copy (12 sites)

- **Files:**
  - `ui/src/modules/rooms/games/kart/KartGame.svelte:342` ('The race could not start.')
  - `ui/src/modules/rooms/games/shooter/ShooterGame.svelte:324` ('The match could not start.')
  - `ui/src/modules/git/GraphView.svelte:1736` (worktree confirm: "Its changes could not be checked…")
  - `ui/src/modules/git/pushFlow.ts:29` ("The force-push target could not be verified…")
  - `ui/src/modules/git/PrList.svelte:90` ('failed to load PRs')
  - `ui/src/modules/personal-agents/AgentEditSheet.svelte:220` ("its schedule could not be added")
  - `ui/src/modules/swarm/GoalsPanel.svelte:74` ('Verification could not start.')
  - `ui/src/modules/rooms/recap-capture.ts:130` and `:149` (capture coverage notes shown in RoomRecap's "Capture coverage notes")
  - `ui/src/modules/browser/live/connection.ts:108` ('…could not reconnect.')
  - `ui/src/modules/workbench/preview/ImagePreview.svelte:99` ('The image could not be decoded.')
  - `ui/src/modules/panels/FileTree.svelte:306` (`Cannot open ${path}: …`)
  - `ui/src/lib/stores/apiStream.svelte.ts:92` ('Could not open relay socket')
- **Evidence:** `content.md:91-93`: "Write 'Couldn’t …', never 'Could not …', in titles, bodies and toasts alike". Iteration 5 fixed the game create and join strings (5-11). These are their neighbours.
- **Fix:** Rewrite each with "couldn’t", for example "The race couldn’t start.", "Couldn’t open {path}: …" and "Couldn’t load pull requests". Capitalise the PrList fallback.

### ux-design-6-09: nit. The Vault OpenAPI viewer shows a bare header for a spec with no operations

- **File:** `ui/src/modules/vault/OpenApiView.svelte:191-206`
- **Evidence:**
  - The view renders the title, version, description and servers, then `{#each [...groups.entries()] …}`.
  - When `spec.paths` is missing or holds no operations (a components-only spec, or a spec split across files), the body is blank below the header, with no message.
  - AGENTS.md: "Design every state: loading, empty, error and loaded".
- **Fix:** After the `#each`, add `{#if groups.size === 0}<EmptyState icon="file" title="No operations in this spec" body="This file defines no paths, so there is nothing to list." />{/if}`, or similar copy.
