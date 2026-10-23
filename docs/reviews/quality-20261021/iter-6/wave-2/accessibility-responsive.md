# Quality iteration 6, wave 2: accessibility and responsive layout (re-score)

Scope: the Svelte 5 UI in `ui/src`, worktree `claude_ade-iter6` (branch `fix/quality-iter6-20261023`), after
fix wave 2. Excluded: `target/`, `node_modules/`, `docs/reviews/`. This was a read-only review using rg, grep, awk,
sed and git, with no builds, tests or e2e runs. It covered the same areas as wave 1 and added a review of
`git diff origin/main -- ui/src` (384 files) and of the wave-2 UI commit `b1c5dae4d` (205 files) for this lens.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 0 | 0.0 |
| minor | 1 (6w2-01) | 0.1 |
| nit | 1 (6w2-02) | 0.03 |

**Score: 10.0 − 0.1 − 0.03 = 9.87 / 10** (wave 1: 9.60, wave 0: 8.87)

Both wave-1 findings are fixed at every listed location. The strict class scan that defined 6w1-01 (`{#if …}` whose
same or next line opens `role="status"` or `aria-live`) now finds 0 status regions in `ui/src` (wave 1: about 80).
The only two hits are persistent regions that wrap an `{#if}` (`shell/Palette.svelte:508`,
`assistant/MemoryTab.svelte:194`), and both are correct. The 52 inserted `role="alert"` lines are kept on purpose:
WebKit speaks an inserted alert, and the policy is written in `accessibility.md`. They are not scored.

Two findings are open:

- **6w2-01** is the rest of the class that the guard cannot see. These are status regions that sit in an
  `{:else if}` or `{:else}` branch, or two or three lines into an `{#if}` block (after a comment or an `{@const}`).
  The `inserted-live-region` detector (`ui/scripts/ui-guards.mjs:979-994`) only matches `{#if` followed by a
  region on the same or the next line. About 18 sites remain. Several are `{:else if}` siblings of sites that this
  wave fixed. Most are loading lines, but two are refresh failures over stale data, and one explains why Publish is
  disabled.
- **6w2-02** is a small side effect of the wave: `announceShown` placed inside a region that is already a mounted
  `role="status"`, so the stop notice is spoken twice.

## Previous findings (wave 1)

| ID | Status | Evidence |
|---|---|---|
| 6w1-01 about 60 status messages inserted with their text | **Fixed** (every listed site) | `LoadState.svelte:67-73` speaks a stale refresh through `announceEdge` (once per distinct error, re-armed on recovery), and `.ls-stale` has no role. The loading region at `:103` exists before its text (`showSkeleton` after the grace, `:107`). Errors use `announceShown`: `RunDetail.svelte:149`, `SharePage.svelte:424`, `ArtifactStage.svelte:270`, `BrandEditor.svelte:542`, `Studio3D.svelte:629`, `DiscoveryChat.svelte:390`, `RefineChat.svelte:338`, `OttoPanel.svelte:594`, `GitAccounts.svelte:396`, `RunMonitor.svelte:74`. Connection and session edges use `announce()` effects: `App.svelte:85-97` (starting, then the assertive "can’t reach the daemon"), `RemoteLiveView.svelte:840-848` (loading, reconnecting, ended), `RoomPage` (`connectionLine`). `GameMatch.svelte:220` and `KartGame.svelte:388` are now always-mounted `sr-only` regions fed at the edge (`KartGame:365` covers the reconnect pause). UsagePage `:551/:562/:1052` and the static truncation notes (`SftpBrowser`, `CsvPreview`, `Lobby`, `MissionControlPage`, `TopicDetail`, `RetentionDialog`, `HistoryList`, `DesignStorage`, `LearnedPage`, `Scorecard`) carry no role. The label-only spinners are `role="img"` with a name: `AnalysisTab.svelte:550`, `HistoryTab.svelte:118`, `OverviewTab.svelte:690`, `Scene3DViewport.svelte:998`. The `inserted-live-region` baseline contains only untoned `role="alert"` lines. The residue the detector misses is carried in **6w2-01**. |
| 6w1-02 TestCasesTab Move buttons disable themselves; a move during a save is lost | **Fixed** | The reorder moved to `product/caseOrder.svelte.ts`. The Move buttons are disabled only at the list edge (`TestCasesTab.svelte:850,859`), not by `saving`. `persist()` (`caseOrder.svelte.ts:72-92`) sets `pending` while a save is in flight, and its `finally` saves again with the current order, so every move is persisted. `move()` (`:98-117`) focuses the pressed button (passed in, because WebKit doesn't focus a clicked button) and falls back to the checkbox only when that button is disabled at the edge. It then announces "Moved to position n of m". `onKey` (`:121-126`) queues the same way and keeps focus on the control that had it. |

Spot checks show that the wave-0 fixes are still in place: `viewportOwnsKey` (`Scene3DViewport.svelte:817`),
LinkPreview Escape (`:29`), `IssueEditor` `role="group"` (`:49`), FileTree `tabRowIn`/`treeKeyAction`
(`:24,261,410`), Inspector `go()` (`:83-88`), and `focusAfterRowRemoval` in AutomationEditor (`:127,155,166`)
and AggregateBuilder (`:178`).

## Areas covered in this iteration

These are the same areas as wave 0 and wave 1:

- Re-verified all fifteen iteration-5 findings (table above).
- Class sweeps across all of `ui/src`:
  - status and live regions inserted together with their text (`{#if …}<x role="status">`);
  - `aria-busy` sites, and hand-rolled loading placeholders;
  - `svelte-ignore a11y_*` suppressions (22 files; the DiffViewer gutter, Markdown, SessionView and PageHeader cases are deliberate and keyboard-reachable);
  - every custom `role="dialog"` popover (WorkflowsPage Node/Runs popovers have focus-in, Esc and return focus, and are clamped; the QueryEditor shortcuts popover is clamped);
  - coordinate-positioned floating UI (vault graph tooltip, vault link preview, NamespacePicker, NotificationBell, GraphView ref popover and ContextMenu all clamp);
  - large fixed widths and minimum widths (every wide table sits in an `overflow-x` wrapper; Onboarding, Login and FirstRunCoach cards cap at the viewport);
  - global reduced motion (the `app.css:844` blanket override plus `lib/motion.ts` for script motion);
  - generic elements carrying `aria-label` (40 sites; mostly harmless container labels or names inside buttons, so not scored);
  - drag-and-drop sources and their keyboard alternatives (15 files; vault FileTree, Kanban, OrgTree, HomeBox, DatabasePage via the connection form folder field, and design-hall LeftPanel all have one).
- Items from iteration 5's "not covered" list:
  - `panels/FileTree` APG gaps (6-06);
  - `api/AutomationEditor` (6-03);
  - `design-hall/site/Inspector` (6-05);
  - `Scene3DViewport` single-key shortcuts (6-08);
  - the Composer slash popup (a listbox driven by `aria-activedescendant` from the textarea, `Composer.svelte:401-439`; the active option scrolls into view; not scored, because a textarea cannot take `role="combobox"`/`aria-expanded`);
  - `MonitorFleet` tables (`role="table"` with `aria-sort`, roving row tabindex and named drill buttons: clean).
- New or recently changed components:
  - `JsonTree`, `ConfluenceSpaceSelect`, `CapsBanner`, `brokers/ResetWarning`, `mcp/ProfilesTab` and `mcp/ProfileEditor`;
  - `product/jira/IssueEditor`;
  - the `classrooms/SchoolBar`, `HoverTag` and `KidCard` components;
  - the shooter HUD (`Vitals`, `TeamPanel`);
  - `vault/LinkPreview` and `previewStore`;
  - `lib/dialogFocus.ts` and `Modal` (focus in, trap, Escape, return: clean).

How the live-region sweep was run this time: the strict wave-1 definition (`{#if` on the line, or on the line
before), then a wider window (`{#if` or `{:else` in the three lines before a `role="status"` or `aria-live`
line). Each of the 35 wider hits was read in context and triaged. A region that exists before its text changes
(a test result that goes from "Testing…" to the result, the pagers, `ClusterWorkspace:488`, `AgentDrivingBar:73`,
`RoomRecap:75`, `RoomPage:176`, `AccountWizard:365`) is fine and not listed.

Wave-2 diff review (`b1c5dae4d`, and `git diff origin/main -- ui/src`):

- 88 `role="status"` lines removed and 2 added. Both added lines are always-mounted regions (`NewSession.svelte:697`,
  `GameMatch.svelte:220`).
- About 64 new `announce()` call sites. Each is an `$effect` gated on a scalar or a string, or a one-shot call in a
  handler. `BrowserSettings.svelte:205`'s `announceShown` changes only on the job state, not on the progress
  percentage. No call site repeats on every poll.
- `announceShown` and `announceEdge` (`lib/announce.svelte.ts:61-91`): edge-detected, re-armed on empty text, and
  the update runs only when the text changes. Clean.
- New components: `S3TransferBars` (announces each transfer once; its Cancel button is a real button),
  `JiraIssueSection` (icon edit buttons have `aria-label` and `title`; the collapsible triggers have
  `aria-expanded`), and the workflows `RunMonitor` split (`RunContextSidebar` collapse and RunMonitor maximize are
  named icon buttons with `aria-pressed`; the run error uses `announceShown`; `RunInputPanel` error is
  announced and referenced by id). `caseOrder.svelte.ts` is covered above.
- No new `svelte-ignore a11y`, no new physical-direction CSS (the only `min-width` lines are moved code), no new
  `position: fixed` or coordinate popups, no clickable non-button elements, and no new keyframes. Transitions use
  `--dur-fast`, which the reduced-motion override covers.

## Findings

### accessibility-responsive-6w2-01: minor. About 18 status regions are still inserted with their text in `{:else if}` branches or a few lines into an `{#if}`, where the `inserted-live-region` guard does not look

`detectInsertedLiveRegion` (`ui/scripts/ui-guards.mjs:979-994`) only flags `{#if` when the same line or the next
line holds a region. In these cases the region is inserted together with its text in the same way. VoiceOver/WebKit
will not speak it, as `lib/announce.svelte.ts:3-8` explains. But the region sits behind `{:else if}` or `{:else}`,
or after a comment line or an `{@const}`. Several are the `{:else if}` sibling of a branch that this wave fixed in
the same block.

- Failures and blocked actions:
  - `modules/design-hall/VersionStrip.svelte:89`: `{#if error}`, then a comment line, then
    `<span class="err" role="status" …> Refresh failed</span>`. A stale-refresh failure is silent. This is the
    `LoadState` stale case that 6w1-01 fixed.
  - `modules/design-hall/LinksPanel.svelte:189`: `{#if error}`, then a comment line, then
    `<div class="pad err" role="status">` "Refresh failed — showing the last good load." Same case.
  - `modules/product/PublishDialog.svelte:474`: `{#if previewStoryId && previewStoryId !== product.selectedId}`,
    then a comment line, then `role="status"` "A different story is open now…". This is the only explanation of why
    Publish is disabled.
  - `modules/product/DiscoveryChat.svelte:396` and `modules/product/RefineChat.svelte:344`:
    `{:else if backgroundTurn || stopNotice}`, then a bubble with `role="status"` and the text "Stopped waiting —
    the agent turn may still finish in the background." (or the stop notice).
  - `modules/assistant/cards/SystemLine.svelte:38`: `{:else if turn?.kind === 'limit'}` inserts
    `<p class="line limit" role="status">` with the limit text. `ChatView`'s own announcer (`:96-112`,
    `:222`) only speaks "working" and "replied".
- Notices that are siblings of fixed branches:
  - `modules/design-hall/brand/BrandEditor.svelte:558`: the `{:else if dirty}` unsaved-changes banner. Its
    `newerHead` sibling at `:542` uses `announceShown`.
  - `modules/design-hall/studio3d/Studio3D.svelte:631`: the `{:else if agentDraft && head}` draft banner ("Showing
    vN draft by Otto · not approved"). Its `agent` sibling at `:629` uses `announceShown`.
  - `modules/design-hall/site/Inspector.svelte:490`: the `{:else if bg?.unknownToken}` warning chip ("The brand
    kit has no …").
- Progress lines and first results:
  - `modules/design-hall/ArtifactStage.svelte:341`: the `{:else if}` "Loading the PDF / 3D model / image…" or "No
    preview yet". Its `diagramError` sibling at `:270` is fixed.
  - `modules/design-hall/ReferencesPanel.svelte:176`: "Searching the library…".
  - `modules/vault/LiveContext.svelte:298`: "Looking for matching workloads…".
  - `modules/design-hall/assist/VariantsTray.svelte:49`: "Drawing…".
  - `modules/agents/conversation/ToolStep.svelte:300`: `aria-live` "Loading the full output…".
  - `modules/aws/AwsPage.svelte:205`: "Checking for the aws CLI…".
  - `modules/browser/live/LiveEngineSetup.svelte:82`: the first install phase. Later phases are spoken.
  - `modules/insights/CapabilitiesPage.svelte:88`: the degraded/missing summary, inserted once loading ends.
  - `modules/help/Walkthroughs.svelte:271`: the first "n guides" count after typing is missed, and only later
    changes are spoken. This is the case that 6w1-01 fixed in `FindInPage`.

Impact (WCAG 4.1.3): a screen-reader user is not told that a design's version list or links failed to refresh
while stale data stays on screen, why Publish is disabled, that Otto stopped waiting for a discovery or refine
turn, or that the assistant hit a limit. The list is much shorter than wave 1's, and most of the remaining sites are
loading lines.

Fix:

1. For the failures and notices, use the pattern that is already in these files:
   `use:announceShown={{ text: '…', assertive: true }}` on the line (or `announce()` at the state edge), and drop
   the role. For example, in `VersionStrip:89` use
   `use:announceShown={{ text: \`Couldn’t refresh the versions. ${error}\`, assertive: true }}`. In `LinksPanel:189`
   do the same. In `PublishDialog:474`, `DiscoveryChat:396`, `RefineChat:344`, `SystemLine:38`, `BrandEditor:558`,
   `Studio3D:631` and `Inspector:490`, use polite `announceShown` with the visible text.
2. For loading lines, drop the role and announce once at the edge, or render `LoadState` / `Skeleton`, which
   already handle this. For `Walkthroughs:271`, keep one always-mounted `sr-only` count region (as in
   `NewSession.svelte:550`) and make the visible count plain.
3. Widen the detector so this class cannot come back: in `detectInsertedLiveRegion`, also match `{:else if` and
   `{:else}`, and look up to two non-comment lines past the opener (skip `<!-- … -->` and `{@const …}` lines).
   Then run `node scripts/ui-guards.mjs --update-baseline`.

### accessibility-responsive-6w2-02: nit (side effect of the wave). The stop notice is spoken twice: `announceShown` inside a `role="status"` bubble that is already mounted

- `modules/product/DiscoveryChat.svelte:365` and `:390`, and `modules/product/RefineChat.svelte:311` and `:338`.
  The "thinking" bubble is a `role="status"` region that is mounted while the turn is in flight. When `stopNotice`
  is then set, the `<p class="stop-notice" use:announceShown={stopNotice}>` inside it is spoken once by the
  bubble's own region (its content changed) and once more by the shell announcer. The wave changed the inner
  `role="status"` to `announceShown`, but the outer region stayed.
- Fix: drop `role="status"` from the thinking bubble. "Thinking…" is already conveyed by
  `LiveWorkingDot`'s label, and the notice is announced by `announceShown`. Alternatively, keep the region and
  remove `use:announceShown` from the inner line. Do one or the other, not both.

## Observed outside the measured areas (not scored)

- `DiscoveryChat.svelte:172-182` and `RefineChat.svelte:152-162`: "Stop waiting" sets `sending = false`, which
  unmounts the bubble that holds the focused button, and "Reload chat" (`reloadAfterStop`) removes itself the same
  way. Focus falls to `<body>`. This is the 6-09 self-removing-control class. Focus should move to the composer, or
  to "Reload chat", after the swap. It is noted for the next pass and not scored, because self-removing controls
  are not one of the swept areas above.
