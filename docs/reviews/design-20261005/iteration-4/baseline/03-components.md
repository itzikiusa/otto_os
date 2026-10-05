## Lens: Shared component usage & quality

**Score: 7.0/10** (10.0 − 7 majors × 0.3 − 8 minors × 0.1 − 3 nits × 0.03 = 7.01). The shared components are well built, but modules still hand-roll empty and error blocks, switches, spinners and pills beside them, and one `role="button"` row has a real keyboard bug.

I read the code in `/Users/itziklavon/claude_ade-design/ui/src` and did not run any builds.

**Credited from the earlier passes (verified):**
- `Modal.svelte` is solid. It restores focus, traps Tab, closes only the top sheet, and ignores backdrop drags that start inside the sheet.
- `Drawer.svelte` has a titled header and a 24 px ✕ (`Drawer.svelte:96-103`), registers with `ui.pushModal()`, and honors reduced motion.
- `Switch.svelte` is a real `<button role="switch">` with RTL-safe travel.
- `LoadState.svelte` follows its documented precedence, with the 150 ms skeleton grace and the stale-data bar.
- `lib/tabKeys.ts` is adopted in about 40 files.
- `EmptyState` has `page`/`panel` variants and an error tone.
- Many pages now use `LoadState` with an `emptyView` snippet (for example `WorkflowsPage.svelte:1879-1887`).

### Findings

1. **[major] Clickable pod row breaks keyboard use**
   - Location: `kubernetes/WorkloadPods.svelte:106` (nested buttons at 117-118).
   - The row is a `div role="button"` whose `onkeydown` handles Enter only. It has no `e.target === e.currentTarget` guard and no Space, so Space does nothing on the row.
   - The row also nests the Logs and Shell `icon-btn`s. Pressing Enter on one of them bubbles to the row and also fires `onopenpod(p.name)`, which opens the pod overview on top of the logs or shell. This breaks the rule against interactive content nested inside a button.
   - Fix: make the name cell a real `<button>` (or an `<a>`) and keep the row non-interactive. At minimum add the target guard and Space handling, as `MonitorOverview.svelte:124` does.

2. **[major] Empty states hand-rolled beside `EmptyState`**
   - Locations:
     - `product/DiscoveryTab.svelte:193` and `product/RefineTab.svelte:185`: `.empty-state` with plain `<p>`s, no icon and no CTA, although the next branch uses `LoadState`.
     - `product/ProductPage.svelte:687`, `aws/AccountRail.svelte:151`, `database/DatabaseChanges.svelte:245`.
     - `workflows/WorkflowsPage.svelte:1831` and `:2082`.
     - `workflows/RunAgents.svelte:236`.
     - `panels/ActivityPanel.svelte:253,257,285,334` and `panels/OutputsPanel.svelte:297` (`.empty-line`).
     - `database/DbAssistantPanel.svelte:155`, `product/MockupAssistPanel.svelte:138`.
   - A grep for the class names finds about 184 matches across 100 files, though not all are real empty states.
   - Rule: components §11 requires `EmptyState`, and it must tell empty apart from filtered-empty and give one CTA.
   - `DatabaseChanges.svelte:245` is also a "Choose a change…" pick-one pane, which breaks the rule that list/detail pages open on an item.
   - Fix: use `EmptyState` (`variant="panel"`, or `compact` text for narrow rails) in each of these.

3. **[major] Error blocks with Retry hand-rolled instead of `LoadState`**
   - Locations:
     - `panels/CanvasPanel.svelte:261`: a custom `.load-error` with its own icon and Retry.
     - `panels/ActivityPanel.svelte:190`, `panels/FileTree.svelte:334`, `git/ConflictFilePane.svelte:224`.
     - `skills-lab/SkillEditor.svelte:241`, `skills-eval/StartEvalForm.svelte:216`.
     - `workflows/WorkflowsPage.svelte:3125` (`.insp-load-err`).
     - `personal-agents/AgentPage.svelte:625,656` (`.err-block`).
     - `api/HistoryList.svelte:199` and `api/CollectionsTree.svelte:304` (`.state.err`).
     - `shell/App.svelte:1098` (`.page-load-error`).
     - `skills-lab/SkillActivity.svelte:156`, `skills-lab/SkillDetail.svelte:346`, `browser/live/LiveEngineSetup.svelte:65,95`, `settings/EmailSenderSetup.svelte:271` (`.inline-error`).
   - Rule: components §11, "use them, don't hand-roll an error block".
   - Each copy words, styles and positions the failure differently, and some lack the stale-data case.
   - Fix: wrap each in `LoadState` (`compact` for rails). Where a one-off banner is intended, use `EmptyState tone="error"` or add a small shared `InlineError` to `lib/`.

4. **[major] Tab-key handling forked despite `lib/tabKeys`**
   - Locations:
     - `kubernetes/ClusterWizard.svelte:183` (`sourceKey`) and `aws/AccountWizard.svelte:169` (`sourceKey`).
     - `product/ProductPage.svelte:407` and `personal-agents/AgentPage.svelte:79` (`onTabKey` copies).
     - `mission-control/MissionControlPage.svelte:41` and `agents/SessionView.svelte:307`.
     - `database/DatabasePage.svelte:761,778`.
     - `brokers/types.ts:86` (`clusterViewKey`), `design-hall/tabKeys.ts`, `aws/util.ts` (`serviceTabKey`).
     - `workbench/EditorTabs.svelte:18`.
   - Each re-implements ←/→/Home/End, so Home/End and wrap behavior can drift between pages.
   - Fix: make `lib/tabKeys.onTabKey` accept an options object (orientation, wrap, a custom `activate`) and delete the forks. Better, build the planned shared `Tabs` component.

5. **[major] Hand-rolled switches next to the shared `Switch`**
   - `Switch` is imported in only 3 files: `mcp/ToolsTab.svelte`, `mcp/ServersTab.svelte`, `vault/VaultPage.svelte`.
   - Local versions:
     - `vault/DocsAgentsView.svelte:558`: a `label.switch` over a hidden checkbox, styled at 1210-1244. There is no `role="switch"`, and the focus ring depends on the invisible input.
     - `workflows/TriggersPanel.svelte:361`: a text-pill `.toggle` that goes green when pressed (556-571).
     - `swarm/SwarmSettings.svelte:311` and `:406`.
     - `kubernetes/monitor/MonitorSettings.svelte:258,307,311`.
     - `design-hall/site/Inspector.svelte:290,530,535`: `checkbox-row toggle`.
   - Rule: components §12 lists `Switch` as built, and foundations reserve green for success.
   - Fix: swap these for `<Switch label=…>`, or for `.checkbox-row` where a checkbox is the right control.

6. **[major] About 28 local spinner rules, and 4 forked spin keyframes**
   - The shared `.spinner` and `otto-spin` live in `app.css:641`.
   - Local ring rules reuse `otto-spin` in 28 places. A sample:
     - `share/SharePage.svelte:541`, `aws/MetricsPanel.svelte:249`, `aws/AccountsOverview.svelte:337`.
     - `panels/FileTree.svelte:565`, `vault/DocsAgentsView.svelte:1720`.
     - `git/LocalReviewPanel.svelte:640,650`, `git/WipPanel.svelte:1359`, `database/ResultsGrid.svelte:1915`.
     - Full list: `grep -rn "animation:.*otto-spin" ui/src --include=*.svelte`.
   - Four sites define their own keyframes, which the guideline explicitly forbids:
     - `vault/RefineDrawer.svelte:388` (`@keyframes spin`).
     - `git/ReviewPanel.svelte:1755,1765` (`@keyframes spin`).
     - `api/ApiPage.svelte:604-606` (`req-tab-spin`).
   - Fix: replace each with `<span class="spinner" style="--spinner-size:…">` inside a `role="status"` wrapper, and delete the local keyframes. The shared class also gets the reduced-motion static ring for free.

7. **[major] Local pill, chip and badge systems**
   - A grep for `.pill`/`.chip`/`.badge`/`.tag`/`.status-pill` rule blocks gives well over 60 matches. Examples:
     - `design-hall/StatusPill.svelte:34` and `assistant/cards/StatePill.svelte:24`. Both implement the full ok/warn/bad/info/neutral set that `StatusBadge`/`McpPill` already provide.
     - `aws/AwsDrawer.svelte:158`, `aws/Ec2View.svelte:407`, `aws/RdsView.svelte:344`, `aws/EksView.svelte:293`.
     - `kubernetes/ResourceTable.svelte:229`, `kubernetes/ResourceDrawer.svelte:467`, `kubernetes/WorkloadPods.svelte:205`, `kubernetes/monitor/MonitorFleet.svelte:870`.
     - `workflows/RunSteps.svelte:509`, `swarm/RunInspector.svelte:469`, `swarm/RunsList.svelte:190`.
     - `swarm/StoryLinkCard.svelte:120-128` (`.chip.good/.accent/.done`).
     - `aws/AccountsOverview.svelte:295-321` (four custom chip tones).
     - `vault/GraphView.svelte:1641` (`.chip.warn`).
   - Hues are chosen at the call site. The guideline wants one pure domain→tone function (`lib/status.ts`) and `StatusBadge`/`McpPill`.
   - Fix: build the planned `Badge` (tone, variant) in `lib/components`, make `StatusPill` and `StatePill` thin wrappers over it, then migrate module by module.

8. **[minor] Nested interactive controls inside `role="button"` containers**
   - Locations:
     - `product/DiscoveryTab.svelte:203-217`: `.run-header` contains a real `<button class="view-swarm-btn">`.
     - `swarm/KanbanBoard.svelte:540-551`: the card contains a checkbox.
     - `kubernetes/monitor/MonitorOverview.svelte:124-135`: the card contains the "Enable monitoring" button.
     - `panels/CanvasPanel.svelte:289-306`: `.ref-body` sits next to an `.icon-btn` group.
   - Rule: components §1, "Put onclick on a div" is banned, and nested interactives confuse assistive tech.
   - Fix: make the header or title a `<button>` (`aria-expanded` for disclosure) and place the other controls as siblings, not descendants.

9. **[minor] Keyboard-unreachable, unlabeled "more" control inside a button row**
   - `git/GraphView.svelte:3376-3391` and the matching branch rows: `<span class="ref-more" role="button" tabindex="-1" title="Tag actions">` holds an icon-only control and is nested in a `div role="button"` row (3360-3371).
   - It has `title` but no `aria-label` and is not in the tab order.
   - Rule: "Real `<button>` controls, `aria-label` + `title` on icon-only buttons."
   - Fix: use `<button class="icon-btn" aria-label="Tag actions" title="Tag actions">` as a sibling of the row's main button. Also verify that the ContextMenu key on the row reaches `tagMenu`.

10. **[minor] Glyph characters used as icons**
    - Locations:
      - `git/RecoveryTools.svelte:167-168`: `↑`/`↓` buttons with `aria-label` but no `title`.
      - `kubernetes/WorkloadPods.svelte:101`: a `↻` table header.
      - `kubernetes/ScaleDialog.svelte:36`: a text `+`.
      - `workflows/WorkflowCanvas.svelte:343`: zoom `+`.
      - `lib/components/Terminal.svelte:2883,2980`: `+`.
      - `canvas/nodes/JsonTree.svelte:52`: `▸`.
    - Rule: `Icon` with a typed `IconName`.
    - Fix: use `<Icon name="plus|arrowUp|arrowDown|refresh|chevronRight">`. Add the missing names to `Icon.svelte` if needed.

11. **[minor] Global classes redefined locally (the `.btn`/`.icon-btn`/`.input` squash trap)**
    - Locations:
      - `vault/DocsAgentsView.svelte:1334-1349`: a scoped `.icon-btn` with a 1 px border, padding and a **red hover** applied to every icon button in that view (used at 527 and 601).
      - `product/OverviewTab.svelte:3205`: a second `.input` (`--surface` fill, 6px 10px padding) that diverges from the shared `.input` (`--surface-2` fill, 27 px).
      - `browser/live/PageDialogCard.svelte:112` (`.input`).
      - `skills-lab/NewSkillModal.svelte:263` (`.input.file`).
      - `aws/SqsView.svelte:767` (`.icon-btn.danger`).
      - `workflows/WorkflowsPage.svelte:4397` (`.btn.danger` redefined).
      - `git/PrDetail.svelte:946` (`.btn.warn`).
      - `agents/FirstRunCoach.svelte:546` (`.btn.big`), `database/QueryEditor.svelte:1895` (`.btn.stop`).
      - `brokers/SchemaVersionsPanel.svelte:252` and `skills-eval/SkillsEvalPage.svelte:597` (`.btn.active`).
    - Fix: delete the overrides. A real `warn` or `active` need becomes one global variant in `app.css`, per "extend it in `lib/`".

12. **[minor] Local form-field system (`.fld`) instead of `.field` and `.input`**
    - 74 uses in 11 files:
      - `scheduled-tasks/ScheduledTasksPage.svelte` (24)
      - `skills-eval/StartEvalForm.svelte` (9)
      - `personal-agents/AgentEditSheet.svelte` (10, with unclassed `<select>`s at 176)
      - `personal-agents/AgentPage.svelte` (9)
      - `vault/DocsAgentsView.svelte` (6)
      - `skills-eval/MatrixView.svelte` (5)
      - `mcp/TokensPanel.svelte`, `mission-control/WorkItemDetail.svelte`, `personal-agents/AgentAutonomy.svelte`, `settings/ConnectionsExport.svelte`, `design-hall/assist/OttoPanel.svelte`
    - Rule: components §2, `.field` plus `.input` give the hint, focus ring and `--surface-2` fill.
    - Fix: migrate these to `.field` and `.input`.

13. **[minor] Hand-rolled loading text**
    - Locations:
      - `shell/Navigator.svelte:1466`, `kubernetes/PodHttpPanel.svelte:219`, `git/FindingsBoard.svelte:289`, `design-hall/LearnedCard.svelte:82`.
      - `panels/CanvasPanel.svelte:259,350`, `panels/OutputsPanel.svelte:286`.
      - `mcp/TokensPanel.svelte:279`: `tokensLoaded ? 'No MCP tokens yet.' : 'Loading…'` with no error branch, so a failed load stays on "Loading…" or reads as empty.
    - Rule: components §11, short specific text and a `Skeleton`/`LoadState`.
    - Fix: wrap these in `LoadState` with a specific `what`.

14. **[minor] The shared `LoadState` headline uses a straight apostrophe**
    - `lib/components/LoadState.svelte:64,70` and `LazyMount.svelte` use "Couldn't", while the copy sweep standardized on "Couldn’t" (`vault/RecoveryView.svelte:117`, `vault/VaultPage.svelte:516`, `personal-agents/AgentEditSheet.svelte:159`).
    - About 217 straight "Couldn't" matches remain across 99 files (`grep -rn "Couldn't" ui/src`), including the visible heading in the component every list page shows.
    - Fix: convert `lib/components` first (`LoadState` and `LazyMount`), then the modules. The e2e `getByText` strings must change in step.

15. **[minor] Drawer logic duplicated outside `shell/Drawer`**
    - `aws/AwsDrawer.svelte:36-54` and `kubernetes/ResourceDrawer.svelte:258-270` each re-implement the phone-sheet branch: `pushModal`, `dialogFocus`, an Esc handler that scans `[aria-modal]`, and a local header.
    - `aws/S3Browser.svelte:736` (a bare `<aside class="drawer">`) and the `drawer-host` at `kubernetes/ClusterWorkspace.svelte:508` are similar.
    - This is a docked desktop column, so it is not necessarily a violation, but the modal half should not be forked.
    - Fix: extract one `DockedDrawer` (inline column on desktop, `Drawer`/`Modal` on phone) in `lib/components` with the header, tabs and close button, and use it for both.

16. **[minor] `aria-label` and `title` differ on the close ✕, the most common icon button**
    - Locations: `lib/components/Modal.svelte:162` ("Close" / "Close (Esc)"), `shell/Drawer.svelte:99,106`, `aws/AwsDrawer.svelte:76`, `kubernetes/ResourceDrawer.svelte:283`.
    - Rule: components §1, `aria-label` and `title` carry the same text.
    - Fix: use `aria-label="Close (Esc)"` or `title="Close"`.
    - Related: `Drawer.svelte:99` builds `Close {title.toLowerCase()}`, which mangles proper nouns such as "AWS" or "Jira".

17. **[nit] `EmptyState` skips the CTA silently and duplicates the error tile**
    - `lib/components/EmptyState.svelte:38` renders the button only if both `actionLabel` and `onaction` are set. A missing `onaction` drops the CTA with no warning.
    - Its error tile (`.empty-icon.error`, line 76) is the same CSS as `LoadState`'s `.ls-icon`.
    - Fix: share one tile, or build the planned `ErrorState` and have both use it.

18. **[nit] `.sheet` class collision risk**
    - `personal-agents/AgentEditSheet.svelte:167` puts `<div class="sheet">` inside a `Modal`. `Modal.svelte:109` finds the top dialog with `.sheet[role="dialog"]`. A role-less `.sheet` is skipped today.
    - Fix: rename the inner class (for example `.form`) so a future role addition cannot break focus trapping.

19. **[nit] Modal is named with `aria-label` instead of `aria-labelledby`**
    - `Modal.svelte:157` repeats `title` in `aria-label` while the `<h2>` already holds the same text.
    - Fix: give the `h2` an id and use `aria-labelledby`.

### What would get this lens to 9.8
- Finish the planned shared pieces and migrate the callers:
  - `Badge` (and remove `StatusPill`/`StatePill` forks)
  - `Tabs` with roving tabindex, built on `lib/tabKeys`
  - `InlineError`/`ErrorState`
  - a docked drawer
  - a `Spinner` or `.spinner` sweep
- Replace every hand-rolled empty, loading or error block listed in findings 2, 3 and 13 with `EmptyState`/`LoadState`. Mark them in `scripts/ui-guards.mjs` (for example "`.empty-state`/`.load-error` class defined" or "`animation: …spin` outside app.css") so the ratchet blocks new ones.
- Replace the `role="button"` rows and cards with real buttons and sibling controls (findings 1, 8, 9), and add an ESLint-style guard for `role="button"` on non-SVG elements.
- Delete the local `.btn`/`.icon-btn`/`.input` overrides and `.fld`, and swap the local toggles for `Switch` (findings 5, 11, 12).
- Replace the glyph icons with `Icon`, and align `aria-label`/`title` on the ✕ buttons (findings 10, 16).
- Make the `Couldn’t` apostrophe change across `lib/components` first, then the modules (finding 14).
