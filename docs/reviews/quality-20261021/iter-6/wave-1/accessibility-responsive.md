# Quality iteration 6, wave 1: accessibility and responsive layout (re-score)

Scope: the Svelte 5 UI in `ui/src`, worktree `claude_ade-iter6` (branch `fix/quality-iter6-20261023`), after
fix wave 1. Excluded: `target/`, `node_modules/`, `docs/reviews/`. This was a read-only review using rg, grep, sed
and git, with no builds, tests or e2e runs. It covered the same areas as wave 0 and added a review of
`git diff origin/main -- ui/src` for this lens.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 1 (6w1-01) | 0.3 |
| minor | 1 (6w1-02) | 0.1 |
| nit | 0 | 0.0 |

**Score: 10.0 − 0.3 − 0.1 = 9.60 / 10** (wave 0: 8.87)

All ten wave-0 findings are fixed at every listed location. The shared announcer (`lib/announce.svelte.ts` and
`Announcer.svelte`, mounted once in `App.svelte:253`) and the new `ui-guards` ratchets (`inserted-live-region`,
`busy-empty-status`, `index-keyed-reorder`) are in place. Two findings are open:

- **6w1-01** is the rest of the 6-01 class. The wave-0 sweep listed about 35 sites. A full scan of the same class
  finds about 60 more that were not listed, including the shared `LoadState` stale banner. FIXES.md lists them as
  "remaining, ratcheted". They are the same defect as the original finding, but not regressions.
- **6w1-02** is a small regression from the 6-04 fix in `TestCasesTab`.

## Previous findings (wave 0)

| ID | Status | Evidence |
|---|---|---|
| 6-01 inserted live regions (~35 sites) | **Fixed** (every listed site) | Each listed site now calls `announce()` at the state edge, and the visible line carries no role. Examples: `settings/InsightsSettings.svelte:36`, `Users.svelte:224`, `Onboarding.svelte:190`, `GitBackup.svelte:129-130`, `ResourceAccess.svelte:156`, `AccountPicker.svelte:50,67`, `OttoUsage.svelte:28-29,165`, `Daemon.svelte:210`, `ContextSoul.svelte:233`, `brokers/TopicDetail.svelte:659-661`, `GroupsTab.svelte:308`, `ResetWarning.svelte:17`, `AutomationEditor.svelte:258`, `ProofPage.svelte:377,1267`, `MultiRunDialog.svelte:384-388`, `UsagePage.svelte:243`, `VaultPage.svelte:62`, `RecoveryTools.svelte:30`, `apiClient.svelte.ts:806` (HistoryList/ApiPanel "Loading request…"), `ResponseViewer.svelte:392`, `Composer.svelte:80`, `ReaderView.svelte:71`, `RoomMedia.svelte:16-17`, `RoomPage.svelte:44-47`, `RoomRecap.svelte:46,51`, `RecapPanel.svelte:19-22`, `GameMatch.svelte:81`, `ShooterGame.svelte:151,605`. The label-only spinners are now `aria-hidden` or `role="img"`: `ApiPage.svelte:416`, `DatabasePage.svelte:1428`, `PrMergeModal.svelte:233`. The class remainder is carried in **6w1-01**. |
| 6-02 busy status placeholders with no text | **Fixed** | `NotificationBell.svelte:370` no longer has a busy wrapper. `ConversationView.svelte:871`, `WorkbenchPage.svelte:573` and `PreviewBody.svelte:204` use `<Skeleton>`. The `busy-empty-status` baseline is 0. |
| 6-03 AutomationEditor / AggregateBuilder index keys, removal focus | **Fixed** | Rows are keyed by stable keys (`AutomationEditor.svelte:310,333,365`, `AggregateBuilder.svelte:203`). Focus follows a move (`:129-140`, `:162-172`), and removal uses `focusAfterRowRemoval` (`:124-127,152-166`, `:174-178`). Row-named Remove buttons are at `:356,378` and `AggregateBuilder:213-215`. |
| 6-04 drag-only reorders | **Fixed** (see 6w1-02 for a side effect) | `TestCasesTab.svelte:163-188` has Move buttons and ⌥↑/⌥↓ at `:934-951`, and the handle is `aria-hidden` (`:890`). `GitTabs.svelte:90-93,113-118` has ⌥⇧←/→ (RTL-aware) and a Move left/right menu at `:131-133`. |
| 6-05 site Inspector drops focus on view swaps | **Fixed** | `Inspector.svelte:83-88` `go()` focuses `[data-view-head]` or the row just left. It is used by the drill-ins (`:446,456`), back (`:347`), Remove block (`:356`), Add (`:442`) and Delete section (`:118-119`). `LinkListField.svelte` keys rows by client id, names each row and moves focus on remove. |
| 6-06 FileTree Tab stop / APG keys | **Fixed** | `panels/treeKeys.ts` `tabRowIn` (the tab stop falls back to the first mounted row) and `treeKeyAction` (↑↓ Home End, →/← child/parent, RTL). The handler at `FileTree.svelte:256-274` scrolls the target into the window before focusing it. |
| 6-07 vault link preview not dismissible | **Fixed** | Escape is handled at `LinkPreview.svelte:29`, and `previewStore.svelte.ts:50-54` sets and clears `aria-describedby`. |
| 6-08 3D shortcuts fire on hover | **Fixed** | The listener is on `host` (`Scene3DViewport.svelte:995`) and is gated by `viewportOwnsKey` (focus only, never a typing target; `viewportLoop.ts:27-29`). |
| 6-09 self-removing controls | **Fixed** | ProfilesTab moves focus to ⋯ (`:81-90,196`). SchoolBar crumbs focus the stage before unmounting (`ClassroomsBox.svelte:631-635`). JsonTree focuses the last revealed row (`:119-129`). CapsBanner calls `focusAppContent()` and its button is named "Dismiss permissions notice" (`:27,41`). ConfluenceSpaceSelect focuses the select or Retry (`:62-66`). GameMatch focuses "Back to games" (`:82,161`). |
| 6-10 IssueEditor group name / late select focus | **Fixed** | `IssueEditor.svelte:49` adds `role="group" aria-label={ef.name}`, and `:32` adds `use:focusOnMount` on the assignee select. |

## Areas covered in this iteration

These are the same areas as wave 0:

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

Wave-1 diff review (`git diff origin/main -- ui/src`, 251 files):

- The new announcer: always mounted, cleared on the next frame. The 43 `$effect(… announce(…))` sites are all
  edge-triggered on a boolean or string, so none repeats on every poll.
- The new components (`LinkListField`, `DoorBubbles` (reduced motion, decorative), shooter `PauseMenu`, `Knocked`
  and `MatchResults` (moved code; Resume still takes focus), `DraftEditor`, `StoryTranscripts`).
- No new `svelte-ignore a11y`. No new icon buttons without `aria-label`/`title`. No new physical-direction CSS,
  except the moved `.br-reload` and a documented `left: 0` projection origin. No new fixed widths or floating
  popups.

## Findings

### accessibility-responsive-6w1-01: major. About 60 more status messages are still live regions inserted together with their text (the rest of the 6-01 class)

Every site that wave 0 listed is fixed, and the `inserted-live-region` ratchet stops new ones. But the same scan
that defines the class (`{#if …}` whose same or next line opens `role="status"`/`aria-live`) still finds about 80
hits in `ui/src`. About 60 of them are real status messages that VoiceOver/WebKit will not speak, for the reason
`lib/announce.svelte.ts:3-8` gives. FIXES.md lists some of them as "remaining, ratcheted". The ones that matter
most:

- Shared component: `lib/components/LoadState.svelte:97-98`. The `ls-stale` banner ("Couldn’t refresh {what} —
  showing the last good load") is inserted with its text, so every page that uses `LoadState` has a silent refresh
  failure.
- Errors and failures:
  - `run-with-otto/RunDetail.svelte:147-148` ("Run failed")
  - `kubernetes/ResourceTable.svelte:153-154` ("refresh failed")
  - `design-hall/ArtifactStage.svelte:268-269`
  - `product/MockupViewer.svelte:243-244`
  - `agents/FirstRunCoach.svelte:263-264` ("Couldn’t check agent CLIs")
  - `agents/NewSession.svelte:697-698` (batch failures)
  - `skills-lab/SkillsBrowser.svelte:401-402`
  - `skills-lab/SkillReviewPanel.svelte:430-431`
  - `help/TourFilm.svelte:185-186`
- Connection and session state:
  - `App.svelte:196-197` (daemon unreachable)
  - `browser/live/RemoteLiveView.svelte:929-930,936-937,962-963` (loading, reconnecting, ended)
  - `share/SharePage.svelte:422-423` (terminal ended)
  - `rooms/games/kart/KartGame.svelte:383,437` (loading; "paused while someone reconnects")
  - `rooms/games/GameMatch.svelte:218` (the lobby region is re-inserted with its text when a match returns to the
    lobby)
  - `brokers/OverviewTab.svelte:96-97`
  - `aws/AthenaView.svelte:536-537`
- Alerts and notices:
  - `usage/UsagePage.svelte:550-551,561-562,1051-1052` (tailer stopped, budget alert, budget exceeded)
  - `workbench/WorkbenchPage.svelte:658-659` (file changed on disk)
  - `design-hall/ArtifactView.svelte:779-780` and `design-hall/brand/BrandEditor.svelte:540-541` (newer version)
  - `design-hall/site/PublishModal.svelte:140-141`
  - `git/ReviewPanel.svelte:1183-1184`
  - `settings/PluginsSettings.svelte:207-208`
- Progress and results:
  - `vault/NoteView.svelte:412-413` ("Saving…")
  - `vault/RefineDrawer.svelte:259-260,262-263`
  - `api/ResponseViewer.svelte:375-376` ("Sending…")
  - `database/SchemaTree.svelte:561-562,579-580`
  - `database/ImportDialog.svelte:277-278`
  - `database/ExportDialog.svelte:288-289`
  - `connections/ConnectionImportDialog.svelte:220-221,279-280`
  - `aws/S3Browser.svelte:664-665,687-688,763-764,771-772`
  - `settings/BrowserSettings.svelte:203-204`
  - `settings/IssueAccounts.svelte:281-282` and `git/GitAccounts.svelte:394-395` (connection test results)
  - `settings/SelfImprovement.svelte:415-416`
  - `design-hall/assist/OttoPanel.svelte:593-594`
  - `design-hall/studio3d/Studio3D.svelte:628-629`
  - `product/DiscoveryChat.svelte:388-389` and `product/RefineChat.svelte:336-337`
  - `lib/components/FindInPage.svelte:431-432` (the first match count is missed)
- Label-only spinners (the same issue 6-01 fixed in ApiPage/DatabasePage):
  - `product/AnalysisTab.svelte:549-550`
  - `product/HistoryTab.svelte:117-118`
  - `product/OverviewTab.svelte:798-799`
  - `lib/design/scene3d/Scene3DViewport.svelte:997-998`
- Static notes, where the role is not useful at all: the truncation and limit notes in `SftpBrowser:363`,
  `CsvPreview:30`, `Lobby:460`, `MissionControlPage:399`, `TopicDetail:803,851`, `RetentionDialog:56`,
  `HistoryList:180`, `DesignStorage:104`, `LearnedPage:458` and `Scorecard:158`.

Impact (WCAG 4.1.3): a screen-reader user isn't told that a refresh failed while the page shows stale data, that
a budget was exceeded, that a live browser or shared terminal is reconnecting or has ended, or that a run failed.

Fix: apply the pattern 6-01 established.

1. `LoadState` first, because it is shared. Call `announce(\`Couldn’t refresh ${what}. ${error}\`)` in an
   `$effect` on the `error` edge and drop `role="status"` from `.ls-stale`.
2. At every other site, call `announce()` at the state edge (assertive for failures and connection loss) and keep
   the visible line plain.
3. For label-only spinners, use `aria-hidden="true"` (the surrounding text or the announcer carries the state) or
   `role="img"`.
4. For static truncation notes, just drop the role.
5. Rerun `node scripts/ui-guards.mjs --update-baseline` so the `inserted-live-region` count goes down.

### accessibility-responsive-6w1-02: minor (regression from the 6-04 fix). TestCasesTab Move buttons disable themselves on every press, so focus always jumps to the checkbox, and a ⌥↑/⌥↓ made during the save is not persisted

- `modules/product/TestCasesTab.svelte:163-180`: `moveCase` calls `void persistOrder()` before `await tick()`.
  `persistOrder` (`:148-150`) sets `savingOrder = true` synchronously, and both Move buttons are
  `disabled={savingOrder || …}` (`:938,947`). So after the tick the button the user pressed is always disabled,
  `keep.disabled` is true (`:179`), and focus goes to the card's checkbox every time. The code comment says this
  should happen only at the top or bottom edge. A keyboard user who presses "Move up" has to Shift+Tab back over
  Accept, Request changes, Edit and Move up (or use ⌥↑ on the checkbox) for every further step.
- The ⌥↑/⌥↓ handler on the checkbox (`:184-188`, wired at `:896`) does not check `savingOrder`. So a second move
  while the first save is in flight reorders `orderedIds` locally, but `persistOrder` returns at
  `if (!activeRun || savingOrder) return;` (`:149`). The new order is never saved, and the cases snap back on the
  next load.
- Fix: read `savingOrder` into the disabled state only for pointer users, or (simpler) keep the buttons enabled
  and queue the save. For example, set a `pendingOrder` flag when `persistOrder` is called while saving, and have
  its `finally` call it again when the flag is set. Then `keep` stays focusable and every move is persisted. Only
  fall back to the checkbox when the edge disables the button that was pressed.
