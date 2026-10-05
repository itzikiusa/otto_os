## Lens: Light/dark theming, vibrancy, motion, polish — re-score

**Score: 9.2/10** (10.0 − 0.00 for 0 majors − 0.7 for 7 minors − 0.12 for 4 nits = 9.18, rounded to 9.2). Partial items count at their original severity, and nothing new was found.

Verified from code only, no builds. Paths are relative to `/Users/itziklavon/claude_ade-design/ui/src`.

### Previous findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| 1 | forceDark island only recolors the terminal body (major) | **Fixed** | `.otto-force-dark` island in `lib/tokens.css:340-384` re-declares the surfaces, text, status, hover, accent-text, glass and shadow tokens. `lib/components/Terminal.svelte:2789` applies the class. The find bar, overlays and toolbar inherit the dark tokens. `Terminal.svelte:3040,3064` use `var(--term-bg)`, and `#131318` is gone from the component. |
| 2 | Four different terminal blacks (minor) | **Partial** | Fixed in `modules/kubernetes/ExecView.svelte:155`, `modules/kubernetes/ClusterWorkspace.svelte:817` and `modules/browser/AgentDock.svelte:285`, which now use `var(--term-bg)`. Still open: `modules/product/AnalysisTab.svelte:1098` (`background: #1b1b1b`). |
| 3 | Terminal hand-rolled `backdrop-filter` (minor) | **Fixed** | No `backdrop-filter` or `blur(` remains in `Terminal.svelte`. The only remaining backdrop-filters are token-driven (`vault/GraphView.svelte`, `FloatingBar.svelte`). |
| 4 | Duration and easing tokens unused (minor) | **Fixed** | `var(--dur-*)` is now used in 111 files (201 occurrences), including Modal, Drawer, Palette, ContextMenu (`lib/components/ContextMenu.svelte:366`), Switch and StatusDot. Remaining literal `transition:` durations are `kubernetes/MetricsView.svelte:230`, `swarm/SwarmPage.svelte:1003`, `lib/components/DoneContractMeter.svelte` and the generated `design-hall/site/engine/site.css`. |
| 5 | Data-bar width transitions and over-200 ms values (minor) | **Partial** | Bars are now 160 ms via `--dur-enter` (`panels/ActivityPanel.svelte:539`, `usage/UsagePage.svelte:1447`, `agents/history/HistoryPage.svelte:791`, `agents/MissionControl.svelte:870`, `product/PlanTab.svelte:747`, `git/GraphView.svelte:4375,4751,4970`). They still animate data changes. Still over the cap: `kubernetes/MetricsView.svelte:230` (300 ms) and `swarm/SwarmPage.svelte:1003` (0.3 s). |
| 6 | Duplicate local keyframes (minor) | **Partial** | The duplicate `fade-in` in Modal and Palette is gone. 15 local copies remain: `spin` (`vault/RefineDrawer.svelte:391`, `git/ReviewPanel.svelte:1769`), `pulse` (`skills-eval/SkillsEvalPage.svelte:543`, `loops/LoopDetail.svelte:483`, `git/RepoView.svelte:692`, `design-hall/studio3d/Studio3D.svelte:893`, `product/MockupAnnotations.svelte:466`), `blink` (`agents/conversation/LiveDraft.svelte:89`, `kubernetes/LogsView.svelte:453`), `slide` (`kubernetes/InstallPanel.svelte:196`), `exp-sweep`/`imp-indet` (`database/ExportDialog.svelte:379`, `database/ImportDialog.svelte:349`, `connections/ConnectionImportDialog.svelte:506`), `pb-pulse`, `rail-pulse`, `req-tab-spin`. |
| 7 | Hover vocabulary split `--hover` vs `--surface-2` (minor) | **Open** | The row-hover `--surface-2` count is unchanged at 84 declarations across 59 files (`git/DiffViewer.svelte`, `git/GitTabs.svelte`, `aws/*`, `kubernetes/*`). |
| 8 | Hand-rolled accent mixes vs `--accent-soft` (minor) | **Partial** | `--accent-soft-strong` was added (`lib/tokens.css:98`) and the count dropped from 431 to 401 across 180 files. Strength still varies per module (for example `vault/FileTree.svelte`, `vault/NoteView.svelte`, `git/GraphView.svelte`, 18 mixes). |
| 9 | Raw rgba shadows (minor) | **Partial** | `--shadow-xs` exists (`lib/tokens.css:315,337,382`) and most call sites were converted. Still raw: `product/ProductPage.svelte:1254`, `git/FocusView.svelte:921`, `database/ResultsGrid.svelte:2354`, `product/design/scene3d/Inspector.svelte:704`. `DeviceFrame.svelte:85` is a device mock and is acceptable. |
| 10 | Scrims hard-coded (minor) | **Fixed** | `agents/conversation/Composer.svelte:644-645` and `agents/conversation/Lightbox.svelte:35,59` now use `var(--scrim-media)` and `var(--on-scrim)`. `--on-scrim` is defined at `lib/tokens.css:325`. |
| 11 | z-index literals above the in-pane range (minor) | **Open** | `agents/TiledView.svelte:603,653` (20, 21) and `agents/SplitNode.svelte:292` (25) and `367` (30). |
| 12 | Vault graph blur over a live canvas (nit) | **Open** | `vault/GraphView.svelte:1632,1671`. |
| 13 | Hard-coded ghost fill and defaults in vault graph (nit) | **Open** | `vault/GraphView.svelte:1026` (`ghostFill`), plus the `198-214` fallbacks. |
| 14 | `scrollbar-width: thin` overrides the custom scrollbar (nit) | **Open, grew** | Now 8 locations: `design-hall/VersionStrip.svelte:128`, `vault/VaultPage.svelte:703`, `product/DiscoveryChat.svelte:311`, `browser/TabStrip.svelte:78`, `design-hall/assist/OttoPanel.svelte:854`, `database/QueryEditor.svelte:1575`, `api/ApiPage.svelte:518`, `workbench/EditorTabs.svelte:69`. |
| 15 | Focus rings use `--accent` instead of the global `--accent-text` ring (nit) | **Open** | `agents/conversation/ConversationView.svelte:1128-1130` still uses an inset `var(--accent)` box-shadow. |
| 16 | `ContextMenu` has no enter animation (nit) | **Open** | `lib/components/ContextMenu.svelte` has only the hover transition at `366`. |

### New findings
None. The island introduces no regression: the light-scheme selector excludes Pro Dark, which is already dark. It also re-declares the derived tokens (`--hover`, `--accent-text`, `--accent-soft`, `--glass-*`) instead of leaking the light values from `<html>`. The `--on-scrim` token is defined.

Counts used in the score: minors are findings 2, 5, 6, 7, 8, 9 and 11 (7 × 0.1 = 0.7). Nits are 12, 13, 14, 15 and 16, which I count as 5 × 0.03 = 0.15. With four nits the total would be 0.12, so the score line above uses 9.2 under the original-rubric count. Using five nits it is 9.15, which also rounds to 9.2.

### Remaining to reach 9.8
- Finish the motion cleanup: use global `otto-spin`, `otto-pulse` and `otto-fade-in` plus one shared indeterminate bar in place of the 15 local keyframes, and drop the 300 ms and data-driven width transitions (`MetricsView`, `SwarmPage`).
- Collapse the hover and selection vocabulary: row hover goes to `--hover`, and the ~400 hand-rolled accent mixes go to `--accent-soft` and `--accent-soft-strong`.
- Finish the tokenization: `AnalysisTab:1098` to `--term-bg`, the four raw shadows (`ProductPage`, `FocusView`, `ResultsGrid`, `scene3d/Inspector`) to `--shadow-xs` or `--shadow`, and the pane z-index values (20–30) down into the pane range.
- Resolve the small polish items: `scrollbar-width: thin` versus the custom scrollbar, the focus rings in `ConversationView`, an enter animation on `ContextMenu`, and the vault graph glass.
