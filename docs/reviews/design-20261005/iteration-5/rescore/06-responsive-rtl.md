## Lens: Responsive & RTL — iteration-5 re-score

**Score: 9.87/10** (arithmetic: 10.00 − 1 minor still open (0.10) − 1 nit still open (0.03) = 9.87)

Judgement: I agree with the arithmetic. I found no regressions.

| # | Iteration-4 item | Status | Evidence |
|---|---|---|---|
| 4 | [minor] Stray media queries (720/1000/768) | Partial | `AgentGraph.svelte:757` is now 640 and `LearnedPage.svelte:591` is now 1024. `agents/history/HistoryPage.svelte:1051` still has `@media (max-width: 768px)`. A full grep of `@media … px` across `ui/src` shows it is the only non-640/1024 width left (`CreatePr:373` is 1025 and `DatabasePage:2803` is 641, both legitimate mirrors). The `max-height: 600px` queries stay gone. |
| 11 | [nit] Graph ports/handles physical | Fixed | `WorkflowCanvas.svelte:343` sets `dir="ltr"` on the viewport, and `QueryBuilder.svelte:942` sets it on `.content`. The physical offsets carry explanatory comments (`WorkflowCanvas:491,497,669,748,751` use `ui-guards: allow`, `QueryBuilder:1751` says "Physical on purpose"). |
| 13 | [nit] Indeterminate sweeps run LTR in RTL | Fixed | `ExportDialog:252` and `kubernetes/InstallPanel:90` both use the shared `.indeterminate`. `app.css:801-808` animates `inset-inline-start`, and the reduced-motion fallback is at `app.css:863-868`. The old `left: -35%` and `translateX(260%)` keyframes are gone from both files. |
| 14 | [nit] Ratchet for non-640/1024 `@media` widths | Open | `ui/scripts/ui-guards.mjs` has no media-width rule (no match for media width or breakpoint), so a new stray query can still land unnoticed. |

Spot-check of items already Fixed (no regressions):
- `reveal-on-hover` has 21 uses across 8 files. The utility is at `app.css`, and `DocsAgentsView` and `FileTree` still use it.
- `StatsTab.svelte:180` still stacks to labelled cells (the `.cl` labels).
- `PoliciesTab:444` and `AllowlistsTab:214` still stack at 640.
- The `MermaidCanvas` and `D2Canvas` controls are still logical.

### New findings
- None.

### Remaining to reach 9.8
- Change `HistoryPage.svelte:1051` from 768 px to 640 or 1024, or to a container query.
- Add a media-width rule to `ui/scripts/ui-guards.mjs` that fails any `@media` width other than 640/1024/641/1025.
