# Mobile and accessibility — owned modules

**Score: 6.4 / 10. Defects: 16.**

Scope was phone and tablet layout plus the accessibility hunts below, only in workflows, scheduled tasks, goal loops, agents (session list, share, handover, composer; PTY drawing skipped), swarm, vault, design hall, canvas, product, proof, run with otto, API client, browser, insights, skills lab, skills eval, plugins, personal agents, assistant, and rooms. Shell, home, panels, git, connections, database, brokers, settings, share, help, and usage were not reviewed.

This is not a 9.8. List/detail pages (scheduled tasks, proof, API, insights, skills, assistant, rooms lobby, personal-agent channels) do push navigation or wrap. The hard gaps are editors and toolbars that still overflow or stay mouse-sized, and a handful of focus rings that throw away the global outline.

## Clean on this pass

- No `window.confirm`, `prompt`, or `alert` calls in these modules. The hits are comments (`vault/FileTree.svelte`, `product/QuestionsTab.svelte`).
- No `font-size` under 11px in these modules’ styles. Readable text uses `--fs-*` (`--fs-xs` is 11px).
- No untokened chrome colours in component `<style>` blocks. Hex that remains is an allowed rendered-page canvas (`ui-guards: allow`) or `design-hall/site/engine/site.css`, which the UI guard excludes because it is the published site’s own brand sheet, not Otto chrome. Game `fillStyle` values in `rooms/games` are the canvas game, not UI chrome.
- CSS animation does not ignore reduced motion. `ui/src/app.css` collapses every animation and transition under `prefers-reduced-motion: reduce`. `product/MockupAnnotations.svelte` pulses a pin and does not add its own query, but it also does not override that global rule. Workflow, swarm, vault, skills-eval, and run-with-otto pulses add a local `animation: none` as well.
- Icon-only `<button>`s sampled in these modules have an accessible name (`aria-label`, usually with `title`). That includes workflow zoom, swarm row menus, scheduled-task overflow, loop back/delete, composer attach/send, canvas assistant close/send, browser tab close, and personal-agent card menus. No missing-name icon button turned up.

## Defects

### 1. Side-docked workflow inspector ignores the viewport

`ui/src/modules/workflows/WorkflowsPage.svelte:1322`, `:2198-2200`, `:2479`, `:4079-4083`.

`ui.wfDockSide` is a persisted preference. On a phone or tablet the editor still sets `padding-inline-end` to the inspector width (resize minimum 280px, reset 400px) and absolutely positions a panel of that width. A 390px screen cannot hold a 400px column; the canvas is crushed and the panel paints past the page.

**Fix:** When `!viewport.isDesktop`, ignore side dock: bottom strip only, no inline padding, no fixed panel width. Keep the preference for the next desktop session.

### 2. Workflow rename field will not shrink

`ui/src/modules/workflows/WorkflowsPage.svelte:4147-4156`.

`.wf-title-edit` is `min-width: 220px` inside `PageHeader`’s `h1`, which is `min-width: 0` and `overflow: hidden`. On a phone the field is clipped instead of filling the title slot, so rename is not usable.

**Fix:** `width: 100%; min-width: 0; max-width: 100%`.

### 3. Running-workflow rows overflow the list pane

`ui/src/modules/workflows/WorkflowsPage.svelte:4008-4063`. The list pane minimum is 220px (`ui/src/lib/paneResizer.ts:27`).

`.run-name` is capped at 120px, but `.run-id` is `flex-shrink: 0` and `.run-when` does not shrink. Padding, gaps, a 6-character id, progress, and a relative time add up to more than 220px (and often more than the 280px default). `.side` does not clip overflow-x, so the row spills across the divider. On a phone the same row is the full first screen and still refuses to shrink.

**Fix:** `min-width: 0` on `.run-row`, let the name take the free space, and drop `.run-id` / `.run-when` under a container query on `.side` (or `overflow: hidden` on the row). Do not keep `flex-shrink: 0` on meta that is not essential.

### 4. Workflow canvas zoom and ports are mouse targets

`ui/src/modules/workflows/WorkflowCanvas.svelte:741-765`, `:787-791`.

Zoom buttons are 26×24px. The connect port is a 12px circle; the comment says the press target is 28px (`::after` inset −8px). Both are named, and both sit on the phone canvas (`touch-action: none`, so the page will not scroll you onto a bigger control).

**Fix:** Under `@media (pointer: coarse)`, make `.zbtn` at least 44×44 and expand `.port.out::after` so the hit box is 44px. Keep the glyphs the size they are.

### 5. Swarm view switcher scrolls off-screen with the scrollbar hidden

`ui/src/modules/swarm/SwarmPage.svelte:966-1023`, `:1162-1187`.

On a phone the switcher is one nowrap row: five tabs, the run/cost budget, and “Max parallel”. `.switcher` is `overflow-x: auto` with `scrollbar-width: none` and a hidden WebKit scrollbar. Tab height is 32px. Budget bars are forced to `flex-wrap: nowrap` and `.cap` is `flex: none; white-space: nowrap`. On a 320px screen “Feed” and the budget are off to the side with no scroll hint.

**Fix:** On `max-width: 640px`, put the tab list on its own row (wrap or equal-width segments, `min-height: 44px`). Put budget and max-parallel on a second wrapping row. If anything still scrolls, leave a visible thin scrollbar or an edge fade. Do not set `scrollbar-width: none` on the only way to reach a tab.

### 6. Vault graph controls can be wider than the phone

`ui/src/modules/vault/GraphView.svelte:1355`, `:1364`, `:1595-1599`, `:1664-1669`.

The panel width is a persisted pixel value (default 210, drag max 560) on an absolutely positioned panel inside `.graph-root { overflow: hidden }`. A width saved on the desktop is clipped on a phone, including the resize handle on the outer edge, so it cannot be dragged back.

**Fix:** Cap the used width with `min(ui.vaultGraphPanelWidth, 100% - 16px)` and, under 640px, present the controls as a full-width sheet instead of a 210px+ overlay.

### 7. Goal-loop criterion kind is a fixed 190px until the phone breakpoint

`ui/src/modules/loops/GoalDefineForm.svelte:472-479`, `:511-524`.

`.kind` is `width: 190px; flex: none`. Wrapping starts only at `max-width: 640px`. A tablet portrait content column (sidebar plus page padding, or one zoom step) is often under 480px, so the verification row (190px select plus the verify field’s min-content) scrolls sideways. Phone itself wraps.

**Fix:** Use the same wrap at `max-width: 1024px`, and change `.kind` to `flex: 1 1 160px; width: auto; max-width: 100%`.

### 8. Composer send and attachment remove stay under 44px

`ui/src/modules/agents/conversation/Composer.svelte:552-557`, `:653-658`.

Send is 28×28. Remove-on-thumbnail is 18×18. Both have names (`aria-label="Send"`, `aria-label="Remove {name}"`). The narrow-container rules (`:728`, `:746`) shrink the textarea; they never grow the buttons. There is no `pointer: coarse` bump.

**Fix:** On coarse pointers, `.send` at least 44×44. Give `.thumb-x` a 44px hit box (padding or a `::before`) without covering the whole thumbnail. “Remove” is also a weak title; use “Remove image” to match the aria-label rule.

### 9. Canvas assistant header controls are ~22px

`ui/src/modules/canvas/ConversationPanel.svelte:247-267`, `:336-343`.

Close, history, and stop are `padding: 4px` around a 14px icon (about 22px). Send is 34×34. They are named. The phone layout correctly replaces the board with this panel (`CanvasPage.svelte:402-410`), so these are the phone controls, and nothing grows them.

**Fix:** `@media (pointer: coarse)` `min-width` / `min-height: 44px` on `.close`, `.hist-btn`, and `.send`.

### 10. Browser reader/live toggles stay 28×30 on a phone

`ui/src/modules/browser/BrowserView.svelte:890-900`, `:948-958`.

The 640px rule grows `.icon-btn.tool` to 36px and wraps the URL field. `.seg` (reader / live) is not in that rule, so those icon-only toggles stay 28×30. They do have `aria-label` and `aria-pressed` (`:689-704`).

**Fix:** In the same 640px block, set `.seg` to at least 44×44.

### 11. API client clear buttons are 20×20

`ui/src/modules/api/HistoryList.svelte:306-308`, `CollectionsTree.svelte:423-426`, `ResponseViewer.svelte:704-706`.

Each overrides `.icon-btn` down to 20×20. Labels exist (“Clear search” and the response-filter equivalent). A phone thumb will miss them.

**Fix:** Delete the 20px size, or under `pointer: coarse` use a 44px hit box. Do not shrink below the shared icon button.

### 12–16. Focus rings removed, replacement too faint to see

The global `:focus-visible` ring in `app.css` loses to a scoped `outline: none`. These replacements are not a visible 2px `--accent-text` ring:

| # | Where | What happens | Fix |
|---|---|---|---|
| 12 | `product/design/scene3d/Hierarchy.svelte:530-536` | Row is `outline: none`. Focus is a 1px inset `var(--accent)` shadow. On `.selected` (`--accent-soft`) that line disappears. | `outline: 2px solid var(--accent-text); outline-offset: -2px` on `:focus-visible`. Drop `outline: none` or restore it in the focus rule. |
| 13 | `product/design/scene3d/Scene3DViewport.svelte:1057-1063` | Viewport is `outline: none`. Focus is `box-shadow: inset 0 0 0 2px var(--accent)`. `--accent` is the fill and fails 3:1 on several dark themes, so keyboard focus on the WebGL view is not reliably visible. | Inset or outer `2px solid var(--accent-text)`. |
| 14 | `product/design/scene3d/Inspector.svelte:449-453` | Name field focus sets `border-color: var(--border)` and `outline: none`. That is the same border as hover, and `--border` is the decorative edge, not `--control-border` / `--accent-text`. | Match `app.css` `.input:focus`: `--accent-text` border and `0 0 0 3px var(--accent-soft-strong)`. |
| 15 | `canvas/SceneList.svelte:249-251` | Search focus is `box-shadow: 0 0 0 1px var(--accent)` (1px of the fill colour). | `border-color: var(--accent-text)` and `0 0 0 3px var(--accent-soft-strong)`. |
| 16 | `workflows/WorkflowCanvas.svelte:545-549` | `:focus` sets `outline: none`. `:focus-visible` only strokes the path with `accent-solid` at 45% opacity. On the dotted canvas that band is easy to miss, and an SVG path has no box for the global ring. | Stroke `var(--accent-text)` at full opacity, `stroke-width: 4` or more, and keep `outline: none` only on that `:focus-visible` rule. |

NumberDrag (`product/design/scene3d/NumberDrag.svelte:139-140`) does this correctly: the input clears its outline and `.nd:focus-within` draws the accent-text halo. Copy that.

## Why 6.4

Naming, type size, token colour, native dialogs, and reduced-motion CSS are in good shape across this set, which is why this is not a failing grade. The score stops there because the workflow editor, swarm switcher, vault graph, goal-loop form, composer, canvas assistant, browser mode toggle, and API clear controls are still desktop-density on a phone, and five focus treatments remove the global ring without putting a contrast-safe one back. Scheduled tasks, proof, insights, skills lab/eval, assistant threads, rooms lobby, and the workflow list’s push navigation are the pages that already do the phone job.
