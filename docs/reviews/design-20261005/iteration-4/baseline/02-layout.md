## Lens: Page layout & toolbar consistency

**Score: 7.8/10** (10.0 − 3 majors × 0.3 − 12 minors × 0.1 − 3 nits × 0.03 = 7.81). Page chrome is mostly unified now. Three non-conforming pages, hand-rolled list-pane resizers and unfinished PageHeader anatomy rules keep it below 9.8.

**Credited from earlier passes, verified in code.**
- PaneDivider with `LIST_PANE` (280 default, 220–420) is used on Proof, Skills, Brokers (sidebar and groups), Swarm, Canvas, Skills Evaluator, Product, AWS, History, Insights and Vault.
- `PageBody fill padded={false}` now wraps most split views.
- The "empty state owns the primary" pattern is applied on Home, Loops, Scheduled Tasks, Workbench, Canvas and Personal Agents.
- Header overflow ordering via `data-overflow` is used well on Swarm, Proof, History and Workflows. Its ⋯ placement and primary-last order are consistent.
- `initialSelection` is used on most list/detail pages (15 files).

### Findings

**[major] Workflows never adopted PageBody and hand-rolls its split**
- `ui/src/modules/workflows/WorkflowsPage.svelte:1671-1759` (`.wf-root > PageHeader + .wf`), resizer at `:1840-1847`.
- The page has no `PageBody`; the body is a self-managed `.wf` with an inline `width:${ui.wfSideWidth}px` (`:1759`). Its list-pane resizer is hand-rolled (270 default, 200–600) instead of `PaneDivider`/`LIST_PANE`.
- This breaks the "PageBody on split views" and `LIST_PANE` rules (layout.md §3.2, §4.1).
- Fix: wrap `.wf` in `<PageBody fill padded={false}>` and swap the resizer for `<PaneDivider bind:width storageKey="workflows.sideW">`. Migrate the stored `ui.wfSideWidth`.

**[major] PR detail has its own title, tab bar and body, with no PageBody**
- `ui/src/modules/git/PrDetail.svelte:422-443` (header and `.prd`), `:456-463` (second title, an `<h2>`), `:479` (second tablist).
- The header title is the generic "Pull request #N". The PR's real title is a second `h2` in the body, so the page has two titles.
- The Summary/Files/Commits tabs sit in a second row below the header.
- Rules cited: layout.md §3.1 (title is the item name; tabs belong in the header) and §3.2.
- Fix: `title={pr.title}` with `#N` and state in `badge`. Move the tabs into the `tabs` snippet. Wrap the body in `PageBody`.

**[major] Skills Lab Evaluator stacks a second tab row under the header tabs**
- `ui/src/modules/skills-lab/SkillsLabPage.svelte:84-97` (header tabs) and `ui/src/modules/skills-eval/SkillsEvalPage.svelte:265-273` (`.se-subhead` with another `segmented` tablist: Golden / Matrix / Runs).
- This is a second toolbar row inside the body, which layout.md §3.1 forbids.
- The list pane also gets its own mini-toolbar (`:282-298`: title, Compare, New).
- Fix: promote the sub-views into the header (shorten the top tabs or use `tabsPlacement="below"`). Keep Compare/New as list-pane icon buttons.

**[minor] Hand-rolled list-pane resizers still bypass `PaneDivider`/`LIST_PANE`**
- `ui/src/modules/api/ApiPage.svelte:330-339` (280, 220–520, plus a legacy `onmousedown={startSideResize}` next to `use:paneResizer`).
- `ui/src/modules/workflows/WorkflowsPage.svelte:1847` (270, 200–600).
- `ui/src/modules/database/DatabasePage.svelte:1068` (`PaneDivider` but with `min={220}`, `defaultWidth={300}`, own max).
- `ui/src/modules/mission-control/MissionControlPage.svelte:413-420` (detail pane: `onpointerdown={startDetailResize}` plus `use:paneResizer`, so two drag implementations on one element).
- The result is three different default widths (270 / 280 / 300) and different clamps for the same role.
- Fix: use `PaneDivider` with the `LIST_PANE` defaults. Allow a documented override only for Database's `sideMaxW()`. Remove the duplicate pointer handlers.

**[minor] Many list panes are fixed-width with no resizer**
- `ui/src/modules/assistant/AssistantPage.svelte:268` (240px list and 260px rail).
- `ui/src/modules/help/Walkthroughs.svelte:335` (272px).
- `ui/src/modules/settings/ContextLibrary.svelte:511` (260px).
- `ui/src/modules/settings/AccessGroups.svelte:492` (180–240).
- `ui/src/modules/workbench/WorkbenchPage.svelte:651,657` (200–250).
- `ui/src/modules/personal-agents/RoomsView.svelte:360` (220px).
- `ui/src/modules/aws/LogsView.svelte:742` (260px).
- `ui/src/modules/api/EnvironmentsView.svelte:306` (220px).
- `ui/src/modules/settings/Settings.svelte:323` (200px nav).
- Widths are 180–272, so the same role changes width page to page. Guideline: list pane is 260–320 and resizable (§4.1; the `PaneDivider` pass).
- Fix: adopt `PaneDivider` and `LIST_PANE` for the page-level list panes (Assistant, Walkthroughs, Workbench, Rooms, ContextLibrary). Settings nav can stay fixed, but document it.

**[minor] Assistant shows the thread list on every tab**
- `ui/src/modules/assistant/AssistantPage.svelte:122,186-195`. `showList` is true on desktop for Tasks, Memory and Permissions too, so a thread list sits beside tabs that don't use it.
- Fix: show the list only on `tab === 'chat'`, or tell the user what the list does on the other tabs.

**[minor] Vault pane maximum breaks `LIST_PANE.max`**
- `ui/src/modules/vault/VaultPage.svelte:39-40,378,466`. Both panes use `max=520` against the 420 shared cap. The left pane's min and default do use `LIST_PANE`.
- Fix: pass `LIST_PANE.max`, or add a named wide-pane constant in `paneResizer.ts` if Vault really needs more.

**[minor] Header buttons mix `.btn` and `.btn.small` for the same role**
- Non-small primaries: `vault/VaultPage.svelte:291`, `git/GitPage.svelte:350`, `aws/AwsPage.svelte:176-181`, `canvas/CanvasPage.svelte:178`, `database/DatabasePage.svelte:949-954`, `personal-agents/PersonalAgentsPage.svelte:147`, `product/ProductPage.svelte:622-637`, `settings/McpServers.svelte:272`, `help/Walkthroughs.svelte:207-213`.
- Small primaries: Proof, Home, Insights, Skills Lab, Scheduled Tasks, Workflows, Users and others.
- `PageHeader.svelte:574-580` fixes height and font size, but only `.btn.small` gets `padding: 0 10px`, so widths still differ.
- Fix: make header controls consistently `.small`. Or have `.ph-actions :global(.btn)` apply the same padding.

**[minor] Secondary action placed after the primary on Product**
- `ui/src/modules/product/ProductPage.svelte:626-640`. Order is Add child, New (primary), Import. The documented order is `[secondary…][primary][⋯]`, and every other page puts the primary last.
- Fix: move Import before New, and give it `data-overflow="-1"`.

**[minor] Git repo header is over the five-control budget**
- `ui/src/modules/git/GitToolbar.svelte:222-288` and `git/GitPage.svelte:327-345`.
- Header controls: Fetch, Pull and its caret, Push (which can turn `.primary` via `pushPrimary`), Branch & stash, ⋯ (data-keep). That is six or seven, plus the inline repo tabs.
- Guideline: five controls or fewer (§3.1).
- Fix: fold Fetch into the Pull split menu, or move Fetch/Pull/Push into `RepoView`'s pane toolbar.

**[minor] Several pages still pass `icon=` to PageHeader**
- `ui/src/modules/workbench/WorkbenchPage.svelte:369`, `rooms/RoomLobby.svelte:50`, `rooms/RoomRecapsPage.svelte:18`, `rooms/RoomPage.svelte:110`.
- layout.md says most pages omit it because the sidebar already shows it. No other module sets it.
- Fix: drop the prop.

**[minor] "Pick a …" panes remain where an item could be preselected**
- `ui/src/modules/skills-lab/SkillsBrowser.svelte:465` ("Pick a skill on the left…"), `aws/SqsView.svelte:357` ("Pick a queue"), `brokers/BrokersPage.svelte:500` ("Pick a cluster").
- Those pages restore a selection, so the pane appears only when nothing was restorable (filter cleared, deleted item). It is better than a bare line but still a pick-one pane.
- Fix: auto-select the first visible row on filter change. The collection summary can be the fallback.

**[minor] The "collapse the list pane" control differs on every page**
- Header `sidebar` icon in `skills-lab/SkillsLabPage.svelte:78` and `canvas/CanvasPage.svelte:167`.
- Accordion chevrons plus a content-toggle in `brokers/BrokersPage.svelte:357,316`.
- In-pane chevron plus a "SCHEMA" rail in `database/DatabasePage.svelte:963-975,1039-1046`.
- A vault `leftOpen` toggle on Vault.
- Fix: one pattern, the header `leading` sidebar icon with `aria-expanded`. Keep the Database rail as an exception if needed.

**[minor] MCP page gutter and clearance differ from PageBody**
- `ui/src/modules/mcp/McpPage.svelte:147` uses `PageBody padded={false}` on a scrolling, non-fill body.
- `mcp/OttoServerHome.svelte:538-539` re-implements the padding (`padding: 18px 20px 40px`), while `mcp/ServersTab.svelte:262` uses `10px 14px` rows.
- The copied padding drops `--fb-clearance`, and the gutters differ between tabs.
- Fix: use a padded `PageBody`. Make the tab bodies full-bleed only where they are real tables.

**[minor] AWS page: error state outside PageBody, and a filter field in the header**
- `ui/src/modules/aws/AwsPage.svelte:196` renders `LoadState variant="page"` directly under the header with no `PageBody`. Compare lines `:189,198,200`.
- The accounts filter input sits in the header actions (`:169-174`), against "don't add filters to the header" (§3.1). It is not marked `data-keep`.
- Fix: wrap the `statusError` branch in `PageBody`. Move the filter into the overview body.

**[minor] Git landing duplicates "Add repository"**
- `ui/src/modules/git/GitPage.svelte:347-352` (header primary), the `+` tab (`GitTabs onadd`, `:320`) and the empty-state action (`:399-401`).
- Rule: the "New" affordance lives once (§4.1).
- Fix: keep the `+` tab (as the page comment says) and drop the header primary.

**[nit] PageHeader gets both `title` and a `titleContent` that repeats it**
- `ui/src/modules/brokers/BrokersPage.svelte:311,328-330`. The title text is rendered twice (title prop plus titleContent).
- Fix: drop the `titleContent` snippet unless the span class is needed.

**[nit] Redundant `width="full"`**
- `ui/src/modules/usage/UsagePage.svelte:445`. It is the default.

**[nit] Providers' header primary is a bulk maintenance action**
- `ui/src/modules/settings/Providers.svelte:427-437` ("Update all CLIs" is `.primary`) on a settings form where nothing else is a primary.
- Fix: make it a secondary action, or consider whether the page needs a primary at all.

Not reviewed in depth, so no finding recorded: `PluginFrame`, `BrowserView` and `KubernetesPage`, which use `PageHeader` without a visible `PageBody`. `BrowserView` looked like an intentional workbench from the grep, but I did not read it.

### What would get this lens to 9.8
- Bring Workflows, PrDetail and the Skills Lab evaluator onto the standard anatomy. Each should have one header, one `PageBody`, the real item as the title, and tabs only in the header.
- Make `PaneDivider` and `LIST_PANE` the only list-pane resizer. Migrate the ApiPage, Workflows, Database and Mission Control hand-rolled ones. Give the fixed 180–272px list panes a resizer, or an explicit documented exception.
- Normalize header buttons: all `.small`, primary last, secondaries before it, and at most five controls (Git, Product).
- Pick one list-pane collapse affordance and use it everywhere. Remove the pick-one fallbacks by auto-selecting.
- Add a guard in `scripts/ui-guards.mjs` that flags `PageHeader` with `icon=`, a `.primary` followed by a non-primary sibling in `actions`, and a `PageHeader` file with no `PageBody`.
