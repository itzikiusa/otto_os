## Lens: Responsive & RTL
**Score: 8.4/10**. Arithmetic: 10.0 − 3 majors (0.9) − 5 minors (0.5) − 6 nits (0.18) = 8.42. Hover-only controls without a touch or keyboard path, one phone table that loses its column labels, and a handful of physical-side leftovers keep it from 9.8.

Verified from the three previous passes:
- **No stray breakpoints in the main tree.** Almost every `@media` width is 640 or 1024. The guideline says "about 17 stray" and only 7 remain (see the minor below).
- **Physical padding/margin shorthands are gone.** My regex found four physical 4-value `padding` shorthands (PageHeader, Terminal, BrowserPanel, Splits) and no 4-value `margin`. `margin: 0 0 0 auto` and `0 auto 0 0` have no matches.
- **RTL handling is in place in the places I checked.**
  - Chat bubbles have an RTL radius flip in `TurnItem.svelte:316` and `ChatView.svelte:369`.
  - Jump pills flip `translateX` in `ChatView.svelte:387` and `ConversationView.svelte:1272`.
  - `RunStageRail.svelte:95` flips its arrow.
  - The `ClusterWorkspace.svelte:126-129` drawer resize reads `direction`.
  - The `Rail.svelte` and `Navigator.svelte` active bars have RTL radii.
- **Coarse-pointer hit areas are solved globally.** `app.css:729-754` grows `.icon-btn`, `.btn`, `.pill-toggle`, chips and segmented buttons with a zero-specificity `::after`. `Navigator.svelte:2413` sets 40 px rows.
- **Wide tables mostly scroll inside their own container.** Examples: `AdminSessions.svelte:320-326`, `Ec2View.svelte:352-356`, and the `MonitorFleet` `.vt` tables.
- **Many hover-reveal controls already have a `(hover: none)` fallback.** About 25 files do, including `GridView`, `GraphView`, `Navigator`, `HomeBox`, `CollectionsTree` (`:555`) and `SkillEditor` (`:427`).

### Findings

[major] Hover-only delete is unreachable by keyboard and touch — `ui/src/modules/vault/DocsAgentsView.svelte:1795-1800` (markup at `:1018`) — `.run-del` is `visibility: hidden` and only becomes visible on `.run-row-wrap:hover`. `visibility: hidden` removes it from the tab order, so it can't be focused. There is no `(hover: none)` or `:focus-within` rule either. This breaks the accessibility rules (visible focus, real controls) and means touch users can't delete a run. Fix: replace the `visibility` toggle with `opacity: 0` plus `.run-row-wrap:hover .run-del, .run-del:focus-visible, .run-row-wrap:focus-within .run-del { opacity: 1 }`, and add `@media (hover: none) { .run-del { opacity: 1 } }`.

[major] Hover-reveal controls with no touch fallback — `ui/src/modules/product/ChatTab.svelte:281-289` (`.archive-btn`), `ui/src/modules/product/RefineTab.svelte:359-366` (`.archive-btn`), `ui/src/modules/product/design/DesignArena.svelte:1267-1274` (`.row-more`, which also lacks a `:focus-visible` reveal), `ui/src/modules/design-hall/site/LeftPanel.svelte:406-415` (`.grip` and `.eye`, `:427-434`), `ui/src/shell/TabBar.svelte:531-550` (`.tab-close`; inactive tabs show no close on touch) — All are `opacity: 0` until `:hover`, with no `@media (hover: none)` override. The sibling files in the same modules have the fallback, so these are the gaps. Fix: add `@media (hover: none) { <selector> { opacity: 1 } }` to each file. Also add `.row-more:focus-visible` to `DesignArena.svelte`, and `.grip:focus-visible` to `LeftPanel.svelte` if it is focusable. Better still, hoist one shared `.reveal-on-hover` utility into `app.css` so a new control can't skip it.

[major] MCP Stats rows lose their column labels on tablet and phone — `ui/src/modules/mcp/StatsTab.svelte:177-184` (markup `:61-81`) — Below 1024 px `.thead` is `display: none` and rows become `1fr 1fr 1fr`. Each cell is then a bare number ("12", "3", "4.2%", "380ms") with no label. Every other stacked table I checked (for example `RunsList.svelte:278`) rebuilds its meaning. Fix: give each numeric cell a `data-label` and show it via `::before` in the stacked mode, or render `<span class="lbl">` inside each cell that is hidden at desktop.

[minor] 7 stray media queries remain — `ui/src/modules/swarm/AgentGraph.svelte:776` (720), `ui/src/modules/design-hall/LearnedPage.svelte:588` (1000), `ui/src/modules/agents/history/HistoryPage.svelte:1036` (768), `ui/src/modules/brokers/BrokersPage.svelte:917`, `ui/src/modules/brokers/TopicDetail.svelte:1449`, `ui/src/modules/brokers/TopicsTab.svelte:640` (all `max-height: 600px`), and `ui/src/modules/git/CreatePr.svelte:373` (`min-width: 1025px`, a legitimate desktop mirror). `layout.md` §5 allows only 640/1024 and says to use a container query for split-pane widths. Fix: move the first three to 640/1024, or to container queries where the width depends on a pane. Decide whether `max-height` is allowed, and if so document it. `rg "@media[^{]*(width|height)[^{]*(720|1000|768|600)px"` finds them.

[minor] MCP tables have no phone layout — `ui/src/modules/mcp/PoliciesTab.svelte:335-345` (7 columns, ~750 px of minimums, no `@media` at all), `ui/src/modules/mcp/AllowlistsTab.svelte:163` — They rely on a bare `overflow: auto` on the wrapper. Rows are separate grids, so on a phone the bottom borders and hover backgrounds stop at the viewport edge while the content scrolls on. The sibling tabs (`ToolsTab:569`, `AuditTab:392`) stack at 640. Fix: stack to cards at 640 like `ToolsTab`, or put the grid in a `min-width` inner wrapper like `AdminSessions` `.table-inner`.

[minor] Canvas floating controls use physical sides — `ui/src/modules/canvas/MermaidCanvas.svelte:585` (`.mode-bar` `left: 12px`), `:626` (`.zoombar` `right: 16px`), `ui/src/modules/canvas/D2Canvas.svelte:588` (`.mode-bar` `left: 12px`) and the matching `.zoombar` — These are UI chrome, not canvas content, so they don't mirror in RTL. Fix: `inset-inline-start: 12px` and `inset-inline-end: 16px`. Keep `.content { left: 0 }` physical because it is the pan origin.

[minor] Two remaining physical-order padding shorthands — `ui/src/lib/components/PageHeader.svelte:593` (`padding: 3px 16px 3px 20px` on `.ph-tabs-below`), `ui/src/lib/components/Terminal.svelte:3179` (`padding: 2px 4px 2px 8px` on `.term-overlay`, whose `inset-inline-end` is already logical) — The padding stays on the physical side while the position flips, so in RTL the PageHeader tab row is misaligned against `.ph-row` (which uses logical `padding-inline: 14px 10px` at `:603`). Fix: `padding-block: 3px; padding-inline: 20px 16px;` and `padding-block: 2px; padding-inline: 8px 4px;`. `BrowserPanel.svelte:710` and `Splits.svelte:210` are the same pattern, but their values are symmetric, so only the form is wrong.

[minor] Small tab controls get no coarse-pointer growth — `ui/src/shell/TabBar.svelte:531-536` (`.tab-close` 16×16), `:597-598` (`.new-tab` 26×22), `ui/src/modules/browser/TabStrip.svelte:122-135` (`.close` 18×18) — TabBar has no `@media` at all. `layout.md` asks for at least 36 px on phone. The global `::after` rule only covers `.icon-btn`, `.btn`, chips and so on, so these bespoke buttons are skipped. Fix: add a `(pointer: coarse)` rule with an `::after { inset: -10px }` hit-area grow, or reuse `.icon-btn`.

[nit] Logical alignment missed in a legend title — `ui/src/modules/settings/AccessGroups.svelte:572` — `float: left` with `width: 100%` on `.detail-title` is the fieldset-legend trick. It is harmless in RTL because it fills the row, but the physical keyword is a smell. Fix: `float: inline-start`, or `display: block`.

[nit] Frame chip anchored to the physical left — `ui/src/modules/canvas/nodes/FrameNode.svelte:76` (`left: 8px`) — Fix: `inset-inline-start: 8px`.

[nit] Graph ports and handles are physically positioned — `ui/src/modules/workflows/WorkflowCanvas.svelte:615,618` (`.port.in` `left: -7px`, `.port.out` `right: -7px`), `ui/src/modules/database/QueryBuilder.svelte:1648,1651` (`left`/`right: -5px`) — This is acceptable only if node graphs are deliberately kept LTR. Make that an explicit rule. If it is intended, add a one-line comment and keep the whole graph `dir="ltr"`.

[nit] Resize-handle line pinned left — `ui/src/modules/database/GridView.svelte:1164` (`.th-resize::after { left: 3px }`) — Check where `.th-resize` is positioned. If it uses `inset-inline-end`, this line is off-centre in RTL. Fix: `inset-inline-start: 3px`.

[nit] Indeterminate progress sweeps run left to right in RTL — `ui/src/modules/database/ExportDialog.svelte:379-380` (`left: -35% → 100%`), `ui/src/modules/kubernetes/InstallPanel.svelte:198-201` and `ui/src/modules/aws/InstallPanel.svelte:129-132` (`translateX(-100% → 260%)`) — Fix: use `inset-inline-start` in the keyframes, or add a `[dir='rtl']` keyframe that negates the translation.

[nit] Chart tooltip offset is physical — `ui/src/lib/components/MetricChart.svelte:344` (`.mc-tip { transform: translateX(8px) }`) — If JS sets `left` from the pointer, flip the offset in RTL, or place the tooltip with a mirrored clamp.

[nit] Skeleton bubble keeps the LTR corner radius — `ui/src/modules/agents/conversation/ConversationView.svelte:1207` (`.sk-user` `border-radius: 18px 18px 5px 18px`) — The real bubble flips in RTL (`TurnItem.svelte:316`) but its skeleton does not. Fix: add the same `:global([dir='rtl']) .sk-user` override.

### What would get this lens to 9.8
- Ship one shared reveal-on-hover utility, or a `lint` rule in `ui/scripts/ui-guards.mjs`. It should fail any `opacity: 0` or `visibility: hidden` control that has no `(hover: none)` and `:focus-visible` counterpart. That clears the touch-fallback finding, the `DocsAgentsView` keyboard hole, `DesignArena`, `LeftPanel`, `ChatTab`/`RefineTab` and `TabBar`.
- Add a stacked-card or labelled-cell phone layout to `StatsTab`, `PoliciesTab` and `AllowlistsTab`. Every table below 1024 px should keep its column meaning.
- Retire the 6 stray breakpoints (720/768/1000, plus the three `max-height: 600px` queries) and add a ratchet that fails any `@media` width other than 640/1024.
- Sweep the remaining physical `left`/`right`/`float: left` and physical-order shorthands (`MermaidCanvas`, `D2Canvas`, `FrameNode`, `PageHeader`, `Terminal`, `AccessGroups`, `GridView`). Document the node-graph ports (`WorkflowCanvas`, `QueryBuilder`) as intentionally LTR.
- Extend the global coarse-pointer hit-area rule, or the shared tab component, to bespoke close and new-tab buttons. Add a 36 px floor for coarse pointers to the `ui/e2e/pages.spec.ts` mobile sweep.
- Add a few RTL e2e cases (canvas toolbars, indeterminate progress, chat skeleton) to `ui/e2e/rtl.spec.ts`, so the remaining physical-side leftovers can't return.
