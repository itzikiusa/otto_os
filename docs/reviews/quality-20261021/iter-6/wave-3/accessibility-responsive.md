# Quality iteration 6, wave 3: accessibility and responsive layout (re-score, final)

Scope: the Svelte 5 UI in `ui/src`, worktree `claude_ade-iter6` (branch `fix/quality-iter6-20261023`), after
fix wave 3. Excluded: `target/`, `node_modules/`, `docs/reviews/`. This was a read-only review using rg, grep, awk,
sed and git, with no builds, tests, guard runs or e2e runs. It covered the same areas as waves 0 to 2 and added a
review of the wave-3 UI commit `c2ba794ee` (82 files under `ui/src`) and the guard commit `1e841dd0d`
(`ui/scripts/ui-guards.mjs`, baselines). No other wave-3 commit touches `ui/src`.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 0 | 0.0 |
| minor | 1 (6w3-01) | 0.1 |
| nit | 1 (6w3-02) | 0.03 |

**Score: 10.0 − 0.1 − 0.03 = 9.87 / 10** (wave 2: 9.87, wave 1: 9.60, wave 0: 8.87)

Both wave-2 findings are fixed at every listed location. All 18 sites of 6w2-01 have lost their inserted role.
Failures and notices now announce through `announceShown` or `announceEdge`. Loading lines are plain text or sit
under an always-mounted Skeleton or LoadState region. The `inserted-live-region` detector now follows `{:else if}`
and `{:else}` and skips comment and `{@const}` lines. 6w2-02 is fixed: neither chat bubble is a region, and the stop
notice is spoken once at its edge.

The score did not move because the fresh pass found two problems in the same areas:

- **6w3-01** is new residue of the same class, beyond the guard's window. The guard reads the opener and then the
  next two lines that mount something. A region on the third mounting line is still silent. The one that matters is
  the agent's "wants to drive Otto" request in `SessionView`. The other three sites are lower impact; two of them
  the guard now flags and the baseline accepts.
- **6w3-02** is a small side effect of this wave. The assistant's limit line now uses `announceShown`, so opening
  a thread speaks an old limit from its history.

## Previous findings (wave 2)

| ID | Status | Evidence |
|---|---|---|
| 6w2-01 About 18 status regions inserted in `{:else if}` branches or a few lines into an `{#if}` | **Fixed** (every listed site; the detector is widened) | Failures, assertive `announceShown` with no role: `VersionStrip.svelte:96` (`refreshFailedText(error)`), `LinksPanel.svelte:190`, `PublishDialog.svelte:309,343,397,452,482`. Notices, polite: `PublishDialog.svelte:477` (`STORY_CHANGED`), `SystemLine.svelte:40`, `BrandEditor.svelte:554,563,570`, `Studio3D.svelte:631`, `Inspector.svelte:490`. `DiscoveryChat`/`RefineChat` after-stop bubbles have no role, and `speakStopNotice` uses `announceEdge` (`DiscoveryChat.svelte:200-204`, `RefineChat.svelte:181-185`). Loading lines are plain: `ArtifactStage.svelte:342`, `ReferencesPanel.svelte:177` (the outcome is spoken at `:179/:181`), `LiveContext.svelte:299` (the section is `aria-busy`), `VariantsTray.svelte:69` (the tray speaks progress through `announceEdge`, `:39-50`), `AwsPage.svelte:208` (`Skeleton label="the AWS console"` is the region), `ToolStep.svelte:303` (`announceShown` only while loading), `CapabilitiesPage.svelte:88`, and `LiveEngineSetup.svelte:87` (the heading speaks each phase, not each percent). `Walkthroughs.svelte:268` is one always-mounted `sr-only` count region; the visible count is plain. Guard: `detectInsertedLiveRegion` (`ui/scripts/ui-guards.mjs:1008-1059`) matches `{#if`, `{:else if}` and `{:else}`, skips blank, comment and `{@const}` lines, and cuts at `{/if}`, `{:else` or `{#snippet`. Untoned alerts move to `inserted-alert`, and `announce-in-region` is a new ratchet (baseline `{}`). The `inserted-live-region` baseline holds two entries (`DiagramView`, `StructureView`); they are carried in 6w3-01. |
| 6w2-02 Stop notice spoken twice (`announceShown` inside a mounted `role="status"` bubble) | **Fixed** | Neither the thinking bubble (`DiscoveryChat.svelte:390`, `RefineChat.svelte:337`) nor the after-stop bubble (`:423` / `:372`) has a role. The inner `<p class="stop-notice">` is plain (`:416` / `:365`). The notice is spoken once by `speakStopNotice()` through `announceEdge`, which is re-armed when the notice clears. The not-scored focus note from wave 2 is fixed as well: `rescueStopFocus` (`DiscoveryChat.svelte:209-213`, `RefineChat.svelte:190-194`) moves focus that fell to `<body>` onto "Reload chat", and `reloadAfterStop` focuses the composer after the reload. |

Spot checks show that the wave-0 and wave-1 fixes are still in place: `viewportOwnsKey`
(`lib/design/scene3d/Scene3DViewport.svelte`), LinkPreview Escape, `IssueEditor` `role="group"`, FileTree
`tabRowIn`/`treeKeyAction`, Inspector `go()`, `focusAfterRowRemoval` (AutomationEditor, AggregateBuilder and the
api env views), `caseOrder.svelte.ts` pending-save queue, and the `app.css:844` reduced-motion override.

## Areas covered in this iteration

These are the same areas as wave 0, wave 1 and wave 2:

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

How the live-region sweep was run this time: the guard's own definition (opener, then the next two mounting
lines), checked by reading `detectInsertedLiveRegion`. Then an rg multiline scan for a region zero to three lines
after an `{#if}` or `{:else}` opener, and a second scan for four to seven lines after, with no `{/if}`, `{:else` or
`{#each}` in between. Each hit was read in context. A region that exists before its text changes is fine and not
listed: `shell/App.svelte:965` (`impAnnounce`), `WipPanel.svelte:885`, `RoomAnnotations.svelte:80` (`kbNote`),
`FloatingBar.svelte:821` (thread log), `AgentDrivingBar.svelte:73`, the `SchoolList.svelte:118` pager, and
`LoadState.svelte:103`. The remaining hits are 6w3-01.

Wave-3 diff review (`c2ba794ee`, `1e841dd0d`):

- **GraphView split** (`git/graph/*`). The ref popover is now `RefPopover.svelte`, lazily mounted on first open
  through `LazyMount quiet`. It keeps `role="dialog" aria-modal`, `dialogFocus` (focus in, trap, Escape, return to
  the expander) and `ui.pushModal`. It is clamped with `clampPopover` (`refPopoverState.svelte.ts:48-57`, never a flip
  without a PAD floor) and capped with `popoverMaxHeight`, and `overflow-y: auto` is set. The row ⋯ buttons carry
  `aria-label`, `title` and `aria-haspopup="menu"`. Branch folders and section headers carry `aria-expanded`.
  `resizerKey.ts` keeps the window-splitter keys and mirrors them under RTL. The phone and tablet accordion survived
  the move of styles into child components. `CommitDetailPane` has its own `mob-visible`/`mob-hidden` rules with
  `min-width: 0`, which overrides the desktop `min-width: 360px`. `RefSidebar` has
  `.refs-panel.mob.mob-collapsed { display: none }` and the 36 px tap targets. GraphView's `:global(.ref-chip)` caps
  still reach `RefChip`. No horizontal-scroll regression was found.
- **Sweep sites.** These were all read: ConversationView `conv-search-scope`, the ToolStep load note, MemoryTab,
  S3Browser, OttoPanel and ConnectionsExport. Self-removing Dismiss and Close buttons call `focusAppContent()`
  (`lib/announce.svelte.ts:42-45`). AccountWizard speaks the connection-test edges. TrustSafety and InsightsPage
  have always-mounted count/progress regions at nesting depth 0. SecretsStore's standing banner is polite. The
  rooms changes were read: `RoomRecap` region always mounted, the `RoomPage` single outage banner with
  `aria-disabled` Reconnect, `controlLine` announced, and the lobby open failure as a toast with an action.
  `OttoServerHome` uses LoadState while the gateway loads.
- **Counts over the diff** (added vs removed lines): `svelte-ignore a11y` +0/−0, `@keyframes` +0/−0,
  physical-direction CSS +0/−0, clickable non-button elements +0/−0, `outline: none` +1/−1 (moved), and `tabindex="-1"`
  +2/−2 (moved). The only new `position: fixed` is the moved popover and its backdrop. Fixed widths of 100 px or
  more: +12/−8, all of them moved popover, sidebar or detail-pane rules or media queries.

## Findings

### accessibility-responsive-6w3-01: minor. Four status regions are still inserted with their text, three or more mounting lines into a branch. The guard misses two of them and accepts the other two in its baseline

`detectInsertedLiveRegion` (`ui/scripts/ui-guards.mjs:1031`) reads the opener line and then at most two lines
that mount something (`taken < 2`). A region on the third mounting line of the branch is mounted together with its
text, like every case already fixed, and WebKit/VoiceOver does not speak it (`lib/announce.svelte.ts:3-8`).

- `modules/agents/SessionView.svelte:973-979`: under `{#if uiPrompt}`, the order is a comment, then
  `<div class="ui-ask" role="group" …>`, then `<span class="ui-ask-mark" …>`, then
  `<span class="ui-ask-text" aria-live="polite">` "… wants to drive Otto — …". This is a permission request with
  Allow and Deny. `uiControl.announce()` (`lib/stores/uiControl.svelte.ts:214-224`, `:217`) speaks it as a toast only when
  the session is not on screen (`if (onScreen) return;`). So a screen-reader user who is looking at the session is
  never told that the agent is asking for control. The guard misses this site, because the region is on the third
  mounting line.
- `App.svelte:195-198`: under `{:else if auth.phase === 'loading'}`, the order is `<div class="boot">`, then
  `boot-mark`, then `<div class="boot-sub" role="status">Connecting to the Otto daemon…</div>`. Wave 2 fixed the
  offline siblings (`:83-97` announces them at the edge). This line kept its role, so it is silent when the phase
  goes from offline back to loading. The guard misses this site too, for the same reason.
- `modules/database/DiagramView.svelte:438-442`: under `{#if graph?.truncated}`, after three comment lines, comes
  `<div class="trunc-banner" role="status">` "Schema capped …". This is a static truncation note. Wave 1 dropped the
  role from the other static truncation notes. The baseline accepts it
  (`ui/scripts/ui-guards-baseline.json:554`).
- `modules/database/StructureView.svelte:636-640`: under `{#if database.objectLoading}`, after a comment, comes
  `<div class="st-head" role="status" aria-live="polite" aria-label="Loading structure">`. This is a loading header
  inserted with its contents. The baseline accepts it (`:555`), and FIXES.md says an e2e spec reads this role.

Impact (WCAG 4.1.3): the one with real impact is the UI-control request. A blind user watching the session is not
told that the agent wants to drive Otto. The other three are a boot line, a static note and a loading header.

Fix:

1. `SessionView.svelte:979`: remove `aria-live`, and add `use:announceShown={`${agentWho} wants to drive Otto: ${uiAskWhat}`}`
   on the `.ui-ask` group (or call `announce()` in the store's `applyEvent` when the request is new and on screen).
2. `App.svelte:198`: remove `role="status"`, and add `announce('Connecting to the Otto daemon…')` on the `loading`
   edge in the existing effect (`:85`).
3. `DiagramView.svelte:442`: remove the role (it is a static note, and the canvas header can say "capped"). In
   `StructureView.svelte:640`, move the region onto an always-mounted wrapper, or render the header plain and let a
   `Skeleton label="the structure"` be the region. Update the e2e spec that reads the role.
4. Guard: count only lines that open an element and skip lines that only close one (`</div>`), or raise `taken < 2`
   to 3 and check that only these four sites appear. Then run `node scripts/ui-guards.mjs --update-baseline --only
   inserted-live-region`, which leaves the baseline empty.

### accessibility-responsive-6w3-02: nit (side effect of the wave). The assistant's limit line now speaks when the thread opens, from its history

- `modules/assistant/cards/SystemLine.svelte:38-40` gained `use:announceShown={turn.text}` on the `limit` line.
  `SystemLine` is rendered for every system card in the thread timeline (`ChatView.svelte:203`) and in task
  entries (`TaskEntry.svelte:34,42`). The action speaks on mount. So when a thread is opened or switched to, every
  past limit line mounts and the newest one is spoken (`announce` keeps the last call per frame), for example
  "Claude usage limit reached …" from days ago. `ChatView`'s own announcer (`:96-113`) resets on a thread switch so
  that "opening an idle thread doesn't announce a reply". This line breaks that rule.
- Fix: remove `announceShown` from `SystemLine`, and announce a limit only when it arrives live. In `ChatView`, add
  an effect that runs when a new `limit` card is appended to the current thread after it has loaded, and skip the
  first render of each thread id (the same `announcedFor` guard it already uses).

## Observed outside the measured areas (not scored)

- `rooms/RoomPage.svelte:183`: Reconnect stays mounted while the reconnect runs, but the whole banner unmounts once
  `connected` is true. If the button had focus, focus falls to `<body>`. This is the self-removing-control class,
  which is not one of the swept areas. A `focusAppContent()` on the connected edge would close it.
