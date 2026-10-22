# Quality iteration 2 — lens: accessibility-responsive

Reviewer scope: Svelte 5 UI (`ui/src`), excluding `ui/src/modules/rooms/games/**`,
`ui/src/modules/home/school/**` and `ui/src/modules/home/boxes/ClassroomsBox.svelte`.
Read-only review (grep/sed/git/python3 text scans only; no builds, tests or e2e).

## Score

Rubric: start 10.0; blocker -1.0, major -0.3, minor -0.1, nit -0.03.

| Severity | Open count | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 1 (2-01) | 0.3 |
| minor | 12 (2-02 … 2-12, plus the still-open part of 1-03) | 1.2 |
| nit | 0 | 0.0 |

**Score: 10.0 - 0.3 - 1.2 = 8.5 / 10**

(Iteration 1 scored 8.7 with less coverage. Eight of nine iteration-1 findings are fixed; the drop
comes from going one level deeper: the Git WIP panel, the side panel, the Assistant chat,
the terminal find bar, MultiRunDialog and the vault tree.)

## Previous findings (iteration 1)

| ID | Status | Evidence |
|---|---|---|
| 1-01 DockedDrawer / Workbench / Logs focus drop | **Fixed** | `lib/components/DockedDrawer.svelte:33-55` remembers the opener and refocuses it when focus is lost with the column; `workbench/WorkbenchPage.svelte:206,216` returns focus to the toggle; `aws/LogsView.svelte:412` refocuses the row |
| 1-02 Room host gets no cue for requests | **Fixed** | `rooms/RoomPage.svelte:135` always-mounted polite region; `:160` host summary + Review in `room-status`; `:167` count + `aria-label` on "People & chat" |
| 1-03 Low-contrast close glyphs / auto-fetch | **Partially fixed (still open)** | QueryEditor fixed. `brokers/BrokersPage.svelte:834-838` `.ctab-x { color: var(--text-dim); opacity: 0.6 }` and `git/GitTabs.svelte:481-486` `.git-autofetch { color: var(--text-dim); opacity: 0.55 }` unchanged (FIXES.md P8 "Not done"). Counted as one open minor |
| 1-04 State only in `aria-label` on generic / `title` | **Fixed** at the cited sites | FloatingBar `:708,716` `aria-describedby` + sr-only; DatabasePage `:1146-1153` tab described by sr-only status; ProofPage `:1039` `role="img"`; Workbench dots `role="img"`. (New sites of the same class found → 2-11, 2-12) |
| 1-05 Workbench validation/autosave chatter | **Fixed** | `WorkbenchPage.svelte:631` `role="status"` banner, `:639` alert only for save errors; `.wb-save` no longer live |
| 1-06 Quick Open combobox | **Fixed** | `workbench/QuickOpen.svelte:91-106` combobox + activedescendant + option ids, no nested button |
| 1-07 DB query tab nests close button | **Fixed** | `database/QueryEditor.svelte:1057` `role="presentation"` wrapper, `:1079` `button role="tab"`, sibling `.qe-tab-close`, Delete closes |
| 1-08 Listbox options in role-less `<li>` | **Fixed** | `design-hall/LinksPanel.svelte:286` and `panels/OutputsPanel.svelte:298` `<li role="none">`, roving tabindex |
| 1-09 Workbench missing from page sweeps | **Fixed** | `ui/e2e/helpers.ts:46` lists `workbench`; `ui/unit/pagesCoverage.test.ts` guards future modules |

## Areas covered (this iteration)

- Re-verification of all nine iteration-1 findings (table above)
- Shell: `App.svelte` phone/tablet chrome (top bar, nav drawer, MobileActionBar, BottomNav), `Navigator.svelte` (session rows, groups, archived list, Telegram/Slack, rail resizer), `Rail.svelte`, `RightPanel.svelte` (collapse, notes, rail toolbar), `SidePane.svelte`, `shell/Drawer.svelte` inline mode
- `lib/rowMenu.ts` keyboard/long-press path, `Modal.svelte` focus trap and return, `Toasts` pause-on-focus
- Agents: `conversation/Composer.svelte` slash popup, `ConversationView` announcements, `ToolStep`/`WorkSteps`/`SubagentCard` disclosures, `SessionView` pane header
- Assistant: `ChatView`, `AssistantPage`, `MemoryTab`, `TasksTab`
- Database: `GridView` grid model (activedescendant, aria-row/colindex), `ResultsGrid` result-set tabs, `MultiRunDialog` (all four stages), `StructureView`, `PlanView`, `QueryBuilder` join badges
- Git: `WipPanel` stage/unstage/discard flow, `GitTabs`, `GitToolbar`, `DiffViewer` gutters
- Kubernetes `PodHttpPanel` (phone grid collapse at 640px), `monitor/MonitorFleet` wide tables (scroll containers), `MetricChart` keyboard slider
- Vault `FileTree` (tree roving tabindex, selection checkboxes), canvas/D2/Mermaid/Snip/RoomAnnotations `role="application"` keyboard paths
- Personal agents `AgentAutonomy`, swarm `KanbanBoard` keyboard move, Home `HomeBox` resize slider
- Repo-wide text scans: `aria-label` on role-less generic elements; toggle buttons without `aria-expanded`/`aria-pressed`; chevron disclosures without `aria-expanded`; `role="tablist"` keyboard models; `role="separator"` keyboard; `requestAnimationFrame` users vs reduced motion; coordinate-positioned popups vs clamping; absolute popups without height caps; fixed widths ≥400 px vs phone scroll containers; all `svelte-ignore a11y_*` suppressions
- Token contrast for `--warning` text used by the new Rooms request summary

## Areas NOT covered yet (next iteration)

- Live regions inserted together with their text (`{#if}<p role="status">…`): app-wide. Only the terminal find bar (2-05) was traced; FindInPage, ChatView's "working" line and LiveStatus need the same check
- Focus after row deletion in list editors (AgentAutonomy goals/rules "Remove", MCP policy rules, settings allow-lists)
- Workflows canvas keyboard model (node move/connect), design-hall Studio3D and product Scene3D viewports
- Product module (OverviewTab, RefineChat, DesignArena) phone layouts
- Settings: SecretsStore, AllowedIdsEditor, ActiveShareLinks, Channels in depth
- `Palette`'s plan mode and `ShortcutsOverlay` with screen reader
- Touch-target sizes (17–22 px glyphs): WCAG 2.2 2.5.8 is outside the 2.1 AA target, so not scored
- Composer slash popup: the `textarea` cannot carry `role="combobox"`/`aria-expanded`, so whether the open state should go through a live region needs a decision. Not scored

## Findings

### accessibility-responsive-2-01 — major — Staging, unstaging or discarding a file in the Git WIP panel drops keyboard focus to `<body>`

- `ui/src/modules/git/WipPanel.svelte:733-738`: the stage checkbox calls `stagePaths([path], section === 'unstaged')`. `:337-348`: `stagePaths` posts and calls `onstatus(s)`, with no focus handling at all (`grep focus` finds only window-focus polling and CSS).
- `:816-833`: unstaged and staged rows are two separate keyed `{#each}` lists (`sectionRows(unstagedRows, …)` at `:912`, `sectionRows(stagedRows, …)` at `:956`). Staging moves the path out of the unstaged list, so Svelte destroys the row together with the focused checkbox. The new row in the other list is a different element.
- The same happens for the folder checkbox (`:779-785`) and for discard (`:756-763` → `discardPaths`). The confirm dialog restores focus to the discard button, and the button is then unmounted by `onstatus`.
- Impact: the core "stage files → commit" flow. Every Space on a checkbox sends a keyboard or screen-reader user back to the top of the document, and nothing announces the result (WCAG 2.4.3, 4.1.3). With a 20-file change set, staging selectively from the keyboard isn't practical.
- Fix: before the call, record the row's section and index (`rows.findIndex(r => r.key === key)`). After `onstatus(s)` + `await tick()`, focus the checkbox at the same index in the same section, or the last row, or the section heading when the list is now empty. Add a polite sr-only region: "Staged src/a.ts". Use the same restore after `discardPaths`.

### accessibility-responsive-2-02 — minor — MultiRunDialog stage changes destroy the focused footer button and never move focus

- `ui/src/modules/database/MultiRunDialog.svelte:733-774`: the footer renders different buttons per `stage` (`setup` → Preview, `preview` → Back/Run, `confirm` → Back/Run, `results` → Stop/New run/Close). `preview()` (`:196`), `proceed()` (`:228`) and `start()` (`:247`) set `stage`, which unmounts the activated button.
- The file has no `focus()`, `tick()` or `data-autofocus` anywhere. `Modal.svelte:103-110` places focus only once, on mount.
- Impact: after "Preview N runs", focus lands on `<body>`, and the next Tab wraps to the modal's first control (`Modal.svelte:150-154`). On the guarded-write `confirm` stage, the typed-phrase input (`:637-646`) is not focused, and screen readers are not told that the dialog content changed (WCAG 2.4.3).
- Fix: after each stage change, `await tick()` and focus the new stage's first meaningful control. Use the typed-phrase input on `confirm`, the `role="progressbar"` region's heading on `results`, and the first section heading (`tabindex="-1"`) on `preview`. Alternatively, add `data-autofocus` per stage and refocus it from an `$effect` keyed on `stage`.

### accessibility-responsive-2-03 — minor — Collapsing the side panel from its own button drops focus

- `ui/src/shell/RightPanel.svelte:366-373`: "Collapse panel" calls `collapse()` (`:130-134`) → `ui.toggleRight()`. `:333-341`: the `<aside>` is then unmounted (`{#if open || browserKept}`) or set `hidden={!open}`, and either way the focused button goes with it.
- In inline mode, `shell/Drawer.svelte:57-64` restores focus only across an inline↔overlay switch. It does nothing on close, and `RightPanel.svelte` has no focus code (only `:148` rail arrows). The same happens on ⌘J while focus is in the notes textarea or a panel control.
- This is the class of 1-01, which was fixed for DockedDrawer but not here. The side panel is on every page.
- Fix: in `collapse()`, focus the active rail button after the toggle (`railEl?.querySelector('.strip-btn[aria-pressed="true"]')` or the one for `shown`), or reuse DockedDrawer's "refocus the opener if focus fell to body" effect.

### accessibility-responsive-2-04 — minor — Side-panel Notes announce "Saving…"/"Saved" on every typing pause

- `ui/src/shell/RightPanel.svelte:435-443`: `.notes-foot` is `aria-live="polite"` and swaps between "Saving…", "Saved" and the idle hint.
- `:272-279`: every `input` sets `saveState = 'saving'` and debounces the flush to 600 ms, then `:258` sets `'saved'`. Each pause while typing produces two announcements over the user's typing.
- This is the same defect as 1-05, which was fixed in Workbench (`.wb-save` is no longer live).
- Fix: drop `aria-live` from `.notes-foot`. Keep the `role="alert"` error branch (`:437`) mounted in an always-present region, so only failures are announced.

### accessibility-responsive-2-05 — minor — Terminal find never announces "No results", and the first count is unreliable

- `ui/src/lib/components/Terminal.svelte:3075-3081`: two different `<span class="find-status" aria-live="polite">` elements sit in an `{#if}/{:else if}`. The live region is created together with its text ("3/12" or "No results"), and screen readers do not reliably announce regions inserted that way. "No results" in particular is always a fresh node. Later changes within the first span ("1/12" → "2/12") do announce.
- Compare `lib/components/FindInPage.svelte:432` (one `aria-atomic` span whose text changes) and `ConversationView.svelte:818` (an always-mounted span).
- Fix: render one always-mounted `<span class="find-status" aria-live="polite" aria-atomic="true">` while the find bar is open, and put the count or "No results" in it as text.

### accessibility-responsive-2-06 — minor — Personal Assistant chat never tells a screen-reader user that Otto replied

- `ui/src/modules/assistant/ChatView.svelte:195-199`: the only status is `{#if working}<p class="working" role="status">…Otto is working…</p>{/if}`. It is inserted with its text and simply removed on completion. The transcript container has no `role="log"`, and `modules/assistant/` has no other live region (grep: only `MemoryTab.svelte:188`).
- The agent ConversationView does this correctly: `modules/agents/conversation/ConversationView.svelte:361-368,981` announces "… finished responding" / "… is waiting for you" through an always-mounted sr-only region.
- Impact: after sending a message, a screen-reader user has to poll the page to learn whether a reply arrived (WCAG 4.1.3).
- Fix: add an always-mounted `<div class="sr-only" aria-live="polite">` and set "Otto is working" / "Otto replied" on the `working` true→false edge, as ConversationView does.

### accessibility-responsive-2-07 — minor — Git repo tabs nest a close button inside `role="tab"`, the tablist has no name, and "uncommitted changes" is colour-only

- `ui/src/modules/git/GitTabs.svelte:232-273`: `<div role="tab" aria-label={label}>` contains `<button class="git-tab-close" tabindex="-1">`. A tab's children are presentational, so this is axe `nested-interactive` (serious). It is the structure 1-07 removed from QueryEditor. The repo convention (BrokersPage, DatabasePage, VaultPage `:486-488`, BrowserPanel `:505`) is a `role="presentation"` wrapper holding tab and close as sibling buttons.
- `:228` `<div class="git-tablist" role="tablist">` has no `aria-label`. Every other tablist in the app is named.
- `:256` `<span class="git-tab-dot" class:dirty aria-hidden="true">`: the amber "uncommitted changes" dot is the only signal, and `aria-label={label}` (`repoTabFit.ts:29-31`, name and branch) does not include it.
- Fix: restructure as `div.git-tab role="presentation"` > `button role="tab"` + sibling close button (keep `tabindex="-1"` and the Delete key). Add `aria-label="Open repositories"` to the tablist. Append ", uncommitted changes" to the tab's accessible name when `isDirty(r.id)`.

### accessibility-responsive-2-08 — minor — Disclosure and selection toggles that expose no state

- `ui/src/shell/App.svelte:1260-1267`: the phone "Open navigator" button toggles `ui.navDrawerOpen` with no `aria-expanded` (or `aria-haspopup="dialog"`). The sibling side-panel button at `:1280-1288` has `aria-expanded`, and BottomNav "More" (`BottomNav.svelte:100`) has both.
- `ui/src/modules/database/StructureView.svelte:1089-1092`: the "DDL" toggle shows open or closed only through the chevron icon. `:794-810`: the index-definition expander, same.
- `ui/src/shell/Navigator.svelte:1389` and `:1422`: the Telegram/Slack `nav-item` buttons toggle `telegramOpen`/`slackOpen` with no state. The twisty next to each has `aria-expanded`, so the same action is two Tab stops and only one of them reports state.
- `ui/src/modules/git/WipPanel.svelte:740-747`: `.wp-name` toggles `selectedPath`, which opens or closes the file's diff, and `class:selected` is not exposed (`aria-pressed`/`aria-expanded` missing).
- Fix: add `aria-expanded={…}` to each disclosure (App, StructureView ×2, the Navigator rows), and `aria-expanded={selectedPath === file.change.path}` (`aria-controls` the diff) on `.wp-name`.

### accessibility-responsive-2-09 — minor — The active session in the Navigator is marked only visually

- `ui/src/shell/Navigator.svelte:1571-1573`: the session row button sets `class:active={… ws.activeSessionId === s.id}` but has no `aria-current`. Module rows in the same component use `aria-current="page"` (`:1109`, `:1215`), as do Rail (`Rail.svelte:154`) and BottomNav.
- Impact: in the main agent list, a screen-reader user can't tell which of N sessions is open (WCAG 1.3.1).
- Fix: `aria-current={!otherWs && router.module === 'agents' && ws.activeSessionId === s.id ? 'true' : undefined}`.

### accessibility-responsive-2-10 — minor — Vault tree selection checkboxes put one extra Tab stop on every note row

- `ui/src/modules/vault/FileTree.svelte:126-131` documents a WAI-ARIA tree with "one row in the Tab order". But `:358-368` renders `<input type="checkbox" class="sel reveal-on-hover">` in every note `treeitem` with the default tab index, so each mounted note row adds a Tab stop between the tree and the next region.
- Its name is the generic "Select for group agent actions", repeated on every row with no note name. `rowKey` (`:149-182`) has no key that toggles selection, so these Tab stops are the only keyboard path to multi-select.
- Fix: `tabindex="-1"` on the checkbox and `aria-label="Select {name}"`. Add a selection key on the row (for example ⌘/Ctrl+Space or `x` → `toggleSelect(path)`), and expose it with `aria-selected`/`aria-checked` on the treeitem or `aria-multiselectable` on the tree.

### accessibility-responsive-2-11 — minor — Loading placeholders and counters put their only text in a prohibited `aria-label` on a generic element

- A text scan for `aria-label` on role-less `div`/`span` (ARIA 1.2 prohibits this, axe `aria-prohibited-attr`, serious) finds loading skeletons whose loading state never reaches assistive technology:
  - `shell/SidePane.svelte:120`, `agents/conversation/ConversationView.svelte:867`, `agents/conversation/PreviewBody.svelte:201`
  - `assistant/ChatView.svelte:137`, `assistant/AssistantPage.svelte:196,216`, `assistant/MemoryTab.svelte:176`, `assistant/TasksTab.svelte:91`
  - `personal-agents/AgentPage.svelte:426,689`, `workbench/WorkbenchPage.svelte:543`, `swarm/SwarmPage.svelte:631`, `product/OverviewTab.svelte:1342`, `git/RepoView.svelte:362`
- `git/GitToolbar.svelte:246-247`: `<span class="ab up" aria-label="{n} ahead">↑{n}</span>`. The label is ignored, so the branch chip reads "↑3 ↓1", and "ahead/behind" exists only in the chip's `title`.
- The correct pattern already exists: `shell/App.svelte:1367` (`role="status"` + label) and `DiffViewer.svelte:1503`.
- Fix: give each loading skeleton `role="status"` (keep `aria-busy`), or put an sr-only "Loading …" text inside it. For GitToolbar, render `<span aria-hidden="true">↑{n}</span><span class="sr-only">{n} ahead</span>`.

### accessibility-responsive-2-12 — minor — A failed statement's result-set tab shows its failure only as a red dot

- `ui/src/modules/database/ResultsGrid.svelte:1373-1386`: `class:err={rs.errored}` plus `<span class="rg-seg-dot" aria-hidden="true">`. The tab's accessible name is "Result {i+1}", and the failing statement appears only in `title`.
- Impact: in a multi-statement run, a screen-reader or colour-blind user can't tell which result set stopped execution (WCAG 1.4.1, 1.3.1). The comment at `:1368-1370` says this dot is the signal for "execution stopped there".
- Fix: append the state to the name, as the 1-04 fixes did: `Result {i + 1}{#if rs.errored}<span class="sr-only">, failed</span>{/if}` (or `aria-describedby` to an sr-only "Statement failed").

### accessibility-responsive-1-03 (carried, still open in part) — minor

- `ui/src/modules/brokers/BrokersPage.svelte:834-838` `.ctab-x { color: var(--text-dim); opacity: 0.6 }` and `ui/src/modules/git/GitTabs.svelte:481-486` `.git-autofetch { color: var(--text-dim); opacity: 0.55 }` still compute to about 2.4–2.8:1 in the light themes (WCAG 1.4.11). The QueryEditor part is fixed.
- Fix: as in iteration 1, drop the opacity (the `aria-pressed` + `.on` accent already show the auto-fetch state).
