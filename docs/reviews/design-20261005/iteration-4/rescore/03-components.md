## Lens: Shared component usage & quality — re-score

**Score: 7.8/10** (10.0 − 5 majors × 0.3 − 6 minors × 0.1 − 2 nits × 0.03 = 7.84). The 5 majors are findings 2, 3, 5, 6 and 7; the 6 minors are 8, 10, 11, 12, 13 and 15; the nits are finding 17 plus one new one. Partial items count at their original severity.

### Status of previous findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| 1 | WorkloadPods `role="button"` row | Fixed | `kubernetes/WorkloadPods.svelte:109-123`: the row is a plain div, the name is a real `<button>`, and Logs and Shell are siblings. |
| 2 | Hand-rolled empty states | Partial | Fixed in DiscoveryTab, RefineTab, AccountRail, DatabaseChanges, ActivityPanel and OutputsPanel (for example `DiscoveryTab.svelte:196-198` now uses `EmptyState`). Still open: `workflows/RunAgents.svelte:236`, `database/DbAssistantPanel.svelte:155`, `product/MockupAssistPanel.svelte:138`, `product/ProductPage.svelte:729,737`. |
| 3 | Hand-rolled error blocks | Partial | Fixed in CanvasPanel (`panels/CanvasPanel.svelte:255` now uses `LoadState`), AgentPage, HistoryList, CollectionsTree, SkillEditor and others. Still open: `git/ConflictFilePane.svelte:224`, `shell/App.svelte:1107`, `skills-eval/StartEvalForm.svelte:219`, `browser/live/LiveEngineSetup.svelte:65,95`, `agents/conversation/ToolStep.svelte:291`. |
| 4 | Tab-key forks | Fixed | `lib/tabKeys.ts:69` now takes options, and the `sourceKey`/`onTabKey`/`serviceTabKey` copies are gone. Only `mission-control/MissionControlPage.svelte:42` remains (new nit below). |
| 5 | Hand-rolled switches | Partial | `DocsAgentsView.svelte:561` and `MonitorSettings.svelte:260,309,313` now use `Switch`. Still open: `workflows/TriggersPanel.svelte:361`, `swarm/SwarmSettings.svelte:311`, `design-hall/site/Inspector.svelte:304,544,549`. |
| 6 | Local spinners and forked keyframes | Open | Still 28 `animation: …spin` rules in 26 files. `@keyframes` remain at `vault/RefineDrawer.svelte:391`, `git/ReviewPanel.svelte:1769` and `api/ApiPage.svelte:588`. |
| 7 | Local pill, chip and badge systems | Partial | `lib/components/Badge.svelte` now exists and `design-hall/StatusPill.svelte:5-27` wraps it. About 70 local `.pill`/`.chip`/`.badge`/`.tag` rule blocks across 64 files remain, for example `assistant/cards/StatePill.svelte`, `aws/Ec2View.svelte` and `workflows/RunSteps.svelte`. |
| 8 | Nested interactives in `role="button"` | Partial | Fixed in `DiscoveryTab.svelte:207-223`, `KanbanBoard.svelte:563` and `CanvasPanel.svelte:269`. Still open: `kubernetes/monitor/MonitorOverview.svelte:124`, a card with a nested "Enable monitoring" button. |
| 9 | GraphView `ref-more` span | Fixed | No `ref-more` or role-button spans remain in `git/GraphView.svelte`. |
| 10 | Glyph icons | Partial | `kubernetes/ScaleDialog.svelte:37` and `WorkloadPods.svelte:101` now use `Icon`. Still open: `git/RecoveryTools.svelte:167-168` (↑ and ↓), `workflows/WorkflowCanvas.svelte:449-450` (+ and −), and `lib/components/Terminal.svelte:2883,2980` (+). |
| 11 | Global classes redefined locally | Partial | The `DocsAgentsView` `.icon-btn` override is gone. Still open: `product/OverviewTab.svelte:3206` (`.input`), `browser/live/PageDialogCard.svelte:112` (`.input`), `workflows/WorkflowsPage.svelte:4476` (`.btn.danger`), and `mcp/PoliciesTab.svelte:433` and `mcp/TokensPanel.svelte:510` (`.btn.danger`). Also open: `git/PrDetail.svelte:915` (`.btn.warn`), `aws/SqsView.svelte:768`, `skills-eval/SkillsEvalPage.svelte:573`, `agents/FirstRunCoach.svelte:546`, `database/QueryEditor.svelte:1895`. |
| 12 | Local `.fld` form fields | Partial | Down from 74 uses in 11 files to 45 in 6 files. Still used in `ScheduledTasksPage` (24), `StartEvalForm` (9), `MatrixView` (5), `TokensPanel`, `WorkItemDetail` and `OttoPanel`. |
| 13 | Hand-rolled loading text | Partial | Only `shell/Navigator.svelte:1473` still has bare "Loading…". |
| 14 | Straight apostrophe in `LoadState` | Fixed | No `Couldn't` remains anywhere in `ui/src`. |
| 15 | Drawer logic duplicated | Open | `aws/AwsDrawer.svelte:43-44` and `kubernetes/ResourceDrawer.svelte:264-265` still carry their own `pushModal` and `dialogFocus`. |
| 16 | Close ✕ label and title mismatch | Fixed | `Modal.svelte:165` and `shell/Drawer.svelte:99,106` now use matching text plus `aria-keyshortcuts`, and the lowercase mangling is gone. |
| 17 | `EmptyState` drops the CTA silently | Open (nit) | `lib/components/EmptyState.svelte:43` still requires both `actionLabel` and `onaction`. |
| 18 | `.sheet` class collision in `AgentEditSheet` | Fixed | `personal-agents/AgentEditSheet.svelte` no longer has `class="sheet"`. |
| 19 | Modal named by `aria-label` | Fixed | `Modal.svelte:160` uses `aria-labelledby={titleId}`. |

### New findings
- **[nit] A tab-key fork remains.** `mission-control/MissionControlPage.svelte:42-47` hand-rolls ←/→/Home/End for a two-tab list/graph toggle. This is the one site not moved to `lib/tabKeys`. Use `onTabKey` there too.

I found no regressions from the new `Badge` or `StatusPill`: `StatusPill` keeps the `.chip` look for its button variant and `Badge` for the plain one.

### Remaining to reach 9.8
- Replace the 28 local spinner rules with `.spinner` and delete the three forked keyframes (`RefineDrawer:391`, `ReviewPanel:1769`, `ApiPage:588`).
- Finish the `Badge` migration (about 70 local pill/chip/badge/tag rules). Replace `StatePill` and the AWS, K8s and workflow pills with `Badge`, and add a ratchet guard.
- Replace the last hand-rolled empty and error blocks: findings 2 and 3 list about 11 sites. Finish the switch swaps (`TriggersPanel`, `SwarmSettings`, `Inspector`) and the `.fld` migration.
- Delete the local `.btn`/`.icon-btn`/`.input` overrides and the remaining glyph icons (`RecoveryTools`, `WorkflowCanvas`, `Terminal`). Un-nest `MonitorOverview.svelte:124`.
- Extract one shared docked drawer for `AwsDrawer` and `ResourceDrawer`. Make `EmptyState` render its CTA, or warn, when `actionLabel` is set without `onaction`.
