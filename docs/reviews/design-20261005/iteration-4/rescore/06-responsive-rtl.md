## Lens: Responsive & RTL — re-score

**Score: 9.8/10** (arithmetic: 10.0 − 1 minor still partial (0.10) − 2 nits still open (0.06) = 9.84)

| # | Previous finding | Status | Verified at |
|---|---|---|---|
| 1 | [major] `DocsAgentsView` run-del hover-only, not keyboard reachable | Fixed | `DocsAgentsView.svelte:1020` uses `reveal-on-hover`. `:1718-1727` adds `:focus-within`, `:focus-visible` and a `(hover: none)` override. |
| 2 | [major] Hover-reveal controls with no touch fallback (ChatTab, RefineTab, DesignArena, LeftPanel, TabBar) | Fixed | The shared `.reveal-on-hover` utility is at `app.css:353-369` (hover, focus-within, focus-visible, `(hover: none)`). It is applied at `ChatTab.svelte:149`, `RefineTab.svelte:212`, `DesignArena.svelte:893`, `LeftPanel.svelte:212,220` and `TabBar.svelte:386`. The local `(hover: none)` rules are at `ChatTab:293`, `RefineTab:373` and `DesignArena:1278`. |
| 3 | [major] MCP Stats rows lose labels at ≤1024 | Fixed | Every cell now carries `<span class="cl">` (`StatsTab.svelte:73-82`). The stacked mode shows the labels (`:199-208`), and the tool name spans the row (`:189-195`). |
| 4 | [minor] 7 stray media queries | Partial | The three `max-height: 600px` queries are gone. 720 (`AgentGraph.svelte:776`), 1000 (`LearnedPage.svelte:589`) and 768 (`HistoryPage.svelte:1037`) remain. |
| 5 | [minor] MCP tables have no phone layout | Fixed | `PoliciesTab.svelte:447-471` stacks to labelled cells at 640 px. `AllowlistsTab.svelte:214` also has a 640 px query. |
| 6 | [minor] Canvas floating controls use physical sides | Fixed | `MermaidCanvas.svelte:585,626` and `D2Canvas.svelte:588,632` are now logical (`inset-inline-start/end`). The pan-origin `left: 0` is kept deliberately. |
| 7 | [minor] Physical-order padding shorthands | Fixed | `PageHeader.svelte:593-594` and `Terminal.svelte:3178-3179` use `padding-block` and `padding-inline`. |
| 8 | [minor] Tab controls get no coarse-pointer growth | Fixed | `TabBar.svelte:586-592` adds a coarse `::after` for the close button. `new-tab` is now an `.icon-btn` (`:438`), which gets the global grow. `TabStrip.svelte:158-164` sets 36 px. |
| 9 | [nit] `AccessGroups` `float: left` | Fixed | `AccessGroups.svelte:574` is `float: inline-start`. |
| 10 | [nit] `FrameNode` chip `left: 8px` | Fixed | `FrameNode.svelte:76` is `inset-inline-start`. |
| 11 | [nit] Graph ports and handles physical | Open | `WorkflowCanvas.svelte:744,747` and `QueryBuilder.svelte:1748,1751` are still `left`/`right`. There is no comment declaring the graph LTR. |
| 12 | [nit] `GridView` resize line `left: 3px` | Fixed | `GridView.svelte:1164` is now `inset-inline-start`. |
| 13 | [nit] Indeterminate sweeps run LTR in RTL | Partial | `aws/InstallPanel.svelte:126,134` is now logical. `ExportDialog.svelte:380` (`left: -35%`) and `kubernetes/InstallPanel.svelte:198-201` (`translateX`) are unchanged. |
| 14 | [nit] `MetricChart` tooltip offset physical | Fixed | The tooltip is positioned by `left:{tipLeft}%` with a `.flip` class (`MetricChart.svelte:277,356`), so the 8 px offset flips with the data side. |
| 15 | [nit] Skeleton bubble keeps the LTR radius | Fixed | `.sk-user` (`ConversationView.svelte:1247-1252`) no longer sets an asymmetric radius, so there is nothing to flip. |

### New findings
- None. I found no regressions. The shared `.reveal-on-hover` selectors are consistent, and the added `(hover: none)` and coarse blocks are scoped correctly.

### Remaining to reach 9.8
- Move the last three stray breakpoints to 640/1024 or to container queries: `AgentGraph.svelte:776`, `LearnedPage.svelte:589` and `HistoryPage.svelte:1037`.
- Make the sweeps in `ExportDialog.svelte:380` and `kubernetes/InstallPanel.svelte:198-201` direction-aware, as `aws/InstallPanel` now is.
- Either add `dir="ltr"` plus a comment to the graph port layers (`WorkflowCanvas.svelte:743-747`, `QueryBuilder.svelte:1747-1751`), or switch them to logical offsets.
- Add a `ui-guards` ratchet for `@media` widths other than 640/1024, so the breakpoint list can't regress.
