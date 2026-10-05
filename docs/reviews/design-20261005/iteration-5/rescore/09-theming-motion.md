## Lens: Light/dark theming, vibrancy, motion, polish — iteration-5 re-score

**Score: 9.77/10** (10.00 − 0.2 for 2 Partial minors − 0.03 for 1 new nit = 9.77)

Checked by grep and read in `/Users/itziklavon/claude_ade-design5/ui/src` (paths below are relative to it), with no builds. The score meets the 9.8 target to within 0.03.

### Findings from the iteration-4 re-score

| # | Item | Status | Evidence |
|---|---|---|---|
| 2 | `#1b1b1b` / `#000` terminal panes | **Fixed** | No `background: #1b1b1b` or `#000` remains in any `*.svelte`; `AnalysisTab` is clean. |
| 5 | Data-bar width transitions; over-200 ms values | **Partial** | The 300 ms outliers in `MetricsView` and `SwarmPage` are gone. Several data bars still animate: `usage/UsagePage.svelte:1463`, `usage/AttributionDrilldown.svelte:337`, `agents/history/HistoryPage.svelte:805`, `agents/MissionControl.svelte:887`. The git detail-pane width/flex transitions are now marked user-driven at `--dur-fast` (`git/GraphView.svelte:4405,4782,5002`) and are fine. |
| 6 | Duplicate local keyframes | **Fixed** | One `@keyframes` is left in `*.svelte`, `git/GraphView.svelte:4540` (`row-pulse`), and it carries a ui-guards allow. The other 14 local copies are gone. |
| 7 | Hover on `--surface-2` vs `--hover` | **Fixed** | Row hover on `--surface-2` fell from 84 declarations to 4 (`kubernetes/WorkloadPods.svelte`, `aws/RdsView.svelte`, `aws/LogsView.svelte`, `aws/S3Browser.svelte`). That is residual, not systemic. |
| 8 | Hand-rolled accent mixes | **Partial** | Down from 401 to 240 occurrences across 140 files. Still systemic, for example `product/OverviewTab.svelte` (12), `git/GraphView.svelte` (5), `workflows/WorkflowsPage.svelte` (7). |
| 9 | Raw rgba shadows | **Fixed** | `ProductPage.svelte:1251`, `database/ResultsGrid.svelte:2337` and `scene3d/Inspector.svelte:705` use `var(--shadow-xs)`. No `rgba(0,0,0` remains in `*.svelte` apart from mask gradients (`agents/conversation/Markdown.svelte:360-361`) and the `DeviceFrame.svelte:85` device mock. |
| 11 | z-index 20–30 | **Fixed** | A grep for z-index of 11 and above finds none in `*.svelte`. |
| 12 | Vault graph blur over canvas | **Fixed** | No `backdrop-filter` remains in `vault/GraphView.svelte`. |
| 13 | Hard-coded ghost fill | **Fixed** | `vault/GraphView.svelte:1029` now uses `theme.dim`. |
| 14 | `scrollbar-width: thin` | **Fixed** | Zero matches in `*.svelte`. |
| 15 | Focus ring colour | **Fixed** | `agents/conversation/ConversationView.svelte:1133-1136` uses `outline: 2px solid var(--accent-text)` with `outline-offset: -2px`. |
| 16 | `ContextMenu` enter animation | **Fixed** | `lib/components/ContextMenu.svelte:354` uses `animation: otto-fade-in var(--dur-enter) var(--ease-out)`. |

### Spot-checks on items already marked Fixed
- **forceDark island:** `lib/tokens.css:368-401` and `lib/components/Terminal.svelte:2793` are intact. `Terminal.svelte` has no `backdrop-filter`.
- **Scrims and shadows:** still tokenized.
- **Duration tokens:** `transition:` literals are now only `design-hall/site/engine/site.css` (generated output, exempt) and `lib/components/DoneContractMeter.svelte:106`.
- **Local `fade-in` keyframes:** still absent.

### New findings
- **[nit] Literal duration on a data meter:** `lib/components/DoneContractMeter.svelte:106` uses `transition: stroke-dasharray 240ms ease-out`. It is over the 200 ms cap, is not tokenized, and animates a data value. Fix: remove the transition or use `var(--dur-enter)`.

### Remaining to reach 9.8
- Drop the width transitions on data bars (`UsagePage`, `AttributionDrilldown`, `HistoryPage`, `MissionControl`) and the `DoneContractMeter` stroke transition. Keep animation only for user-driven pane resizing.
- Codemod the remaining ~240 hand-rolled `color-mix(in srgb, var(--accent) N%, transparent)` fills to `--accent-soft` and `--accent-soft-strong`, or add a ui-guards ratchet. `product/OverviewTab.svelte` and `workflows/WorkflowsPage.svelte` are the biggest.
- Finish the last four `--surface-2` row hovers (`WorkloadPods`, `RdsView`, `aws/LogsView`, `S3Browser`) so every row hover uses `--hover`.
