# Quality iteration 1 — lens: ux-design

Reviewer: ux-design (static, read-only; no builds/tests run). Base: `fix/quality-loop-20261021` @ `d447516c4`.
Scope excluded: `ui/src/modules/rooms/games/**`, `target/`, `node_modules/`, `docs/reviews/`.

## Score

Rubric: start 10.0; blocker −1.0, major −0.3, minor −0.1, nit −0.03.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 5 | 0.50 |
| nit | 1 | 0.03 |
| **Total** | 7 | **0.83** |

**Score: 10.0 − 0.83 = 9.17 / 10**

## Previous-finding status

None. This is iteration 1 of this loop. Earlier campaigns
(`docs/reviews/quality-next-20261008/design-ux-iteration4.md`) closed with no open
design findings. This pass did not reopen any of them.

## Areas covered

- PageHeader/PageBody presence on every `modules/*/*Page.svelte`. Pages without
  them are embedded sub-pages (Capabilities, SkillsEval, Share) or route
  switchers whose children render their own header (Kubernetes, Design Hall).
- Multiple `.primary` actions per PageHeader (one-pass scan). The only hits were
  mutually exclusive branches in Swarm, Personal agent and History.
- Raw exceptions used as toast titles. All non-literal titles are constants or
  human labels.
- Literal hex colours. The only hits are documented exceptions: provider brand
  marks, xterm and three.js/excalidraw JS options, and the Appearance theme
  previews.
- Hand-rolled overlays (`position: fixed`, `100vh`, `role="dialog"`). They clamp
  into the viewport, except the DiffViewer `100vh` (finding 06).
- Icon-only `.icon-btn` buttons without `aria-label`. None found.
- Toggle buttons that combine `aria-pressed` with a label that flips with state
  (finding 03).
- List/detail opening on an item (GroupsTab, History and others auto-select).
- Outward-facing confirms (`confirmOutward` / `confirmProd`): Jira transition,
  comment, assign and watch; Product question post and rewrite publish; Finding →
  issue; SQS send, delete, purge and redrive. All name where, what and who.
- Rooms, excluding games: RoomPage, RoomLobby, RoomRecapsPage, StartRoomModal,
  RoomRecap, RoomParticipants, RoomChat, RoomMedia and RoomScreens. The states,
  confirms and copy were reviewed.
- Recent features: Linear issue tracker UI (#107), Channels AllowedIdsEditor
  (#106), the git "landed in develop / main" mark (eb295dcc2), Side panel
  (RightPanel), and IssueAccounts.
- ui-guards baseline debt, reviewed for context. Ratcheted debt was not
  re-reported, except where it causes a concrete defect.

## Areas NOT covered yet (next iteration)

- Database Explorer deep states: QueryEditor, ResultsGrid, ImportDialog and
  StructureView error/empty paths.
- Git: PrDetail composer "Posts to PR #N" line, CreatePr confirm, WipPanel
  conflict flows, FocusView.
- Workflows editor inspector (3.5k-line WorkflowsPage), TriggersPanel copy.
- Product: DesignArena, DiscoveryChat, RefineChat (heavily changed in 452ade172).
- Loops (LoopDetail +261 lines), Scheduled tasks (+331 lines), Swarm GoalsPanel
  and RunGraph.
- Settings: Providers / providerForm, AccessGroups, Channels full form.
- Kubernetes ClusterWorkspace, ResourceDrawer, PodHttpPanel (prod guardrails).
- AWS AthenaView, LogsView, S3Browser states.
- Help/walkthroughs, Home boxes, Assistant cards, plugins (PluginFrame).
- Every module's phone/tablet layout. This is static only; no screenshots were taken.

---

## Findings

### ux-design-1-01 — major — Attached issue shows a dead "Open in browser" link and an empty status chip

- **Files:**
  - `ui/src/modules/agents/AttachIssue.svelte:21-28`
  - `ui/src/modules/agents/JiraIssuePicker.svelte:154`
  - `ui/src/modules/panels/InfoPanel.svelte:207-217`
- **Evidence:**
  - The picker emits only `{ account_id, key, summary }` (`onpick({ account_id: selectedAccountId, key: issue.key, summary: issue.summary })`). It drops the `url` and `status` that `IssueSummary` carries (`types.ts:4369-4374`).
  - `AttachIssue` then persists `url: ''` and `status: ''` into `session.meta.issue`. It also hard-codes `provider: 'jira'`, even for a Linear issue.
  - No daemon code fills these fields; a `rg` for `meta.issue` / `"issue"` in `crates` finds no enrichment.
  - The session Info panel renders `<a class="issue-key" href={attachedIssue.url} target="_blank" title="Open in browser">`. That link has an empty href, so it opens the app's own URL or nothing.
  - The panel also renders `<span class="chip">{attachedIssue.status}</span>` as an empty pill.
  - This breaks the loaded state of a core-path panel: the primary affordance says "Open in browser" and does not open the issue.
- **Fix:**
  - Widen `PickedIssue` to carry `url`, `status` and the account's provider. Pass `issue.url` and `issue.status` in `onpick`.
  - In `AttachIssue`, store them and the real provider. Widen `AttachedIssue.provider` to `'jira' | 'linear'` in `types.ts` and the contract.
  - In `InfoPanel`, render the key as plain text when `url` is empty, and omit the chip when `status` is empty. This covers sessions that already hold the empty values.

### ux-design-1-02 — minor — Linear rollout left Jira-only copy on shared issue surfaces

- **Files:**
  - `ui/src/modules/agents/AttachIssue.svelte:39` (`<Modal title="Attach Jira issue">`)
  - `ui/src/modules/panels/InfoPanel.svelte:203` ("Jira issue" section title) and `:230` ("Attach Jira issue…")
  - `ui/src/modules/git/ReviewPanel.svelte:1165,1168,1302,1305,1775` ("Attach Jira story…", "Remove Jira story", Modal "Attach Jira story")
  - `ui/src/modules/git/FindingActions.svelte:256` ("Convert to Jira issue…" / "In Jira as {key}"), `:358` (`aria-label="Jira project key"`) and `:70` ("Creating the Jira issue…")
- **Evidence:**
  - The shared picker is explicitly "Jira or Linear, per account" (`JiraIssuePicker.svelte:2`, and its empty state "No Jira or Linear accounts configured.").
  - `FindingActions.submitJira` already computes `tracker = 'Linear' | 'Jira'` for its confirm and toast.
  - The surrounding labels still say Jira. A Linear user reads "Attach Jira issue", picks a Linear issue, and is then told "In Jira as ENG-123".
- **Fix:**
  - Use tracker-neutral copy: "Attach issue…", "Issue", "Attach story…", "Convert to issue…", "Project or team key".
  - Name the tracker only where the account is known, for example `In ${trackerName} as ${key}`. Store the provider as described in finding 01.

### ux-design-1-03 — minor — Toggle buttons flip their accessible name while also exposing `aria-pressed`

- **Files:**
  - `ui/src/modules/rooms/RoomMedia.svelte:41`
  - `ui/src/modules/rooms/RoomScreens.svelte:23`
  - `ui/src/shell/RightPanel.svelte:358-363`
  - `ui/src/modules/workflows/WorkflowsPage.svelte:2518-2524`
  - `ui/src/modules/agents/Splits.svelte:164-170`
  - `ui/src/modules/panels/BrowserPanel.svelte:571-580`
  - `ui/src/modules/browser/ReaderView.svelte:232-241`
  - `ui/src/modules/design-hall/studio3d/Studio3D.svelte:614-616`
  - `ui/src/modules/product/PlanTab.svelte:562-564`
  - `ui/src/modules/vault/NoteView.svelte:440-446`
  - `ui/src/modules/design-hall/site/SiteCanvas.svelte:546`
  - `ui/src/modules/workbench/FileList.svelte:245-248`
  - `ui/src/modules/aws/S3Preview.svelte:170`
- **Evidence:**
  - Each button sets `aria-pressed={state}` and also swaps its name to the opposite verb. Example from RoomMedia: `aria-pressed={room.annotations_enabled !== false}` with text `'Disable annotations'` when pressed.
  - Other examples:
    - RightPanel: `aria-label={ui.rightWide ? 'Restore panel width' : 'Expand panel'} aria-pressed={ui.rightWide}`
    - Splits: `aria-pressed={broadcastMode}` with "Exit broadcast"
    - PlanTab: "Hide raw"
    - Studio3D: "Stop"
    - FileList: "Back to files"
    - S3Preview: "Table" when raw
    - SiteCanvas: "Show on mobile" when hidden
  - VoiceOver then announces contradictions such as "Disable annotations, toggle button, pressed".
  - The repo fixed exactly this defect class in `a3bac6526` ("keep the hidden-files toggle name stable"). `accessibility.md` §toggles requires `aria-pressed` to expose the state.
- **Fix:** For each site, either keep the name stable and keep `aria-pressed` (for example "Annotations", "Wide panel", "Broadcast", "Raw", "Play", "Trash", "Hidden on mobile"), or keep the verb swap and drop `aria-pressed`. Put the verb in `title` only if it is useful.

### ux-design-1-04 — minor — Room's first connect is rendered as an error state with a Retry action

- **File:** `ui/src/modules/rooms/RoomPage.svelte:138`
- **Evidence:**
  - `connection` starts as `'connecting'` (line 25), and `attach()` runs on mount.
  - While `!room`, the page renders one `EmptyState`:
    - title: `connection === 'connecting' ? 'Connecting to the room…' : 'Connection interrupted'`
    - body (always): "Terminal access is paused until your room connection is restored."
    - action (always): `actionLabel="Retry"`
  - So every normal open briefly shows error copy ("paused… restored") and a Retry button while the first connection is in flight. The guidelines require distinct, designed loading and error states: specific loading text, and Retry only on failure.
- **Fix:**
  - Split the two states. While `connection === 'connecting'`, show a loading state (`LoadState loading what="the room"` or a Skeleton) with no action.
  - Show the "Connection interrupted" EmptyState with Retry only for `disconnected`. Disable Retry while a reconnect is pending.

### ux-design-1-05 — minor — "Retry speech capture" is always shown while a recap is capturing

- **File:** `ui/src/modules/rooms/RoomRecap.svelte` (host block): `{#if room.recap.state === 'capturing'}<p class="hint">Capturing … </p><button class="btn small" onclick={() => capture?.resumeAudio()}>Retry speech capture</button>{/if}`
- **Evidence:**
  - The Retry button is rendered unconditionally in the capturing state, including when speech capture is healthy.
  - `captureStatus` already exposes `audioSources` and `gaps`, but neither gates the button.
  - The host sees a recovery action with no failure, next to "Pause capture" / "Finish recap". That misstates the state and crowds the action row.
- **Fix:** Show "Retry speech capture" only when speech is degraded, for example `captureStatus.audioSources === 0` or a speech gap is recorded. Pair it with the reason.

### ux-design-1-06 — minor — DiffViewer file navigator is capped at `100vh` (WKWebView screen-height trap)

- **File:** `ui/src/modules/git/DiffViewer.svelte:1615-1632` (`.diff-nav { position: sticky; top: 0; max-height: 100vh; … }`)
- **Evidence:**
  - `components.md:278-279` says "`100vh` there is the *screen* height". The review checklist (§2 Components) says custom floating UI "never relies on `100vh`".
  - The navigator is sticky inside the diff's scroll container, which is shorter than the screen because of the toolbar, PageHeader and tabs.
  - `100vh` therefore lets the nav extend below the visible scrollport. Its inner `.nav-files` scroller cannot bring the last files into view until the diff body is scrolled to its end.
- **Fix:** Cap it against the scroll container instead. Use `max-height: 100%` on a stretched flex column, or a container-relative variable that the component measures, as the other popovers do with `window.innerHeight`.

### ux-design-1-07 — nit — Room recaps sub-page navigates back via a header action instead of crumbs

- **File:** `ui/src/modules/rooms/RoomRecapsPage.svelte:18` (`<button class="btn small" onclick={() => router.go('rooms')}>Rooms</button>` in `actions`)
- **Evidence:**
  - `layout.md:183` defines `crumbs` for sub-pages (`Kubernetes / Monitor / Fleet`). Kubernetes Monitor uses `crumbs={[{ label: 'Kubernetes', … }]}`.
  - Room recaps (`#/rooms/recaps`) is a sub-page of Rooms, but it spends a header action slot on navigation. That makes "Rooms" look like a verb next to "Refresh" and "Local engines…".
- **Fix:** `<PageHeader title="Room recaps" crumbs={[{ label: 'Rooms', onclick: () => router.go('rooms') }]} …>` and drop the button.
