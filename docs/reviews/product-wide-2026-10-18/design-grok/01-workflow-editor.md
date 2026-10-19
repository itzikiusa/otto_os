# Workflow editor — design review

Score: **8.4** / 10. Defects: **10**.

Target was 9.8. Not met. The list, empty, error, retry, approval, and stop flows are implemented. The canvas still paints catalog hex over the theme accent, and the editor column clips on a phone. No browser pass. No Cargo, npm, or engine edits.

## Defects

1. `ui/src/modules/workflows/WorkflowCanvas.svelte:90` and `:393`, stripe at `:678`. Each node sets `--accent` to the catalog hex (`#f0c040`, `#9aa0aa`, `#d97cff`, …). The stripe, selected border, hover border, and ports follow that hex, not the user’s accent, and they do not change between light and dark. Gold and gray nodes fail on a light surface. **Fix:** do not assign `color()` to `--accent`. Keep selection, ports, and focus on the theme accent. If a kind needs a mark, use one semantic token (`--info` / `--warning` / `--accent-text`), never the catalog string.

2. `ui/src/modules/workflows/WorkflowsPage.svelte:2209` and `.pal-ic` at `:4202`. Add-node icons use the same hex as `color` on an 18% tint. Condition, loop, human approval, delay, and transform icons are nearly invisible in light mode. **Fix:** drop `--c`; paint the icon with `--accent-text` on `--accent-soft`.

3. `ui/src/modules/workflows/WorkflowCanvas.svelte:539`. Hovering a connection paints the hit path with `--status-exited` (the failure red). `.edge-hit:hover + .edge` never matches, because the visible path is the previous sibling, so the thick transparent hit stroke turns red on top of the curve. **Fix:** stroke `--accent` (or `--border-strong`). Put the hit path before the visible path if the sibling selector is kept.

4. `ui/src/modules/workflows/WorkflowsPage.svelte:2093` and `:2293`. Opening Run… leaves the header button `.btn.primary` and adds a second primary Run in the form. **Fix:** while the form is open, the header control is a pressed secondary (not `.primary`). The form’s Run stays the only primary, and while the POST is in flight its label is `Starting…`.

5. `ui/src/modules/workflows/WorkflowsPage.svelte:4065` (`.main`), `:4225` (canvas `min-height: 0`), `:4242` (inspector `flex-shrink: 0`), `:4796` (`.run-input` has no max-height). `PageBody` `fill` is `overflow: hidden`. Run input is ~480px and the inspector will not shrink, so on a phone the canvas goes to 0 and the inspector (or the bottom of a taller form) is clipped with no scroll. **Fix:** give `.main` `overflow-y: auto`, give `.canvas-wrap` a floor (`min-height: 160px`), and cap `.run-input` with `max-height` plus `overflow-y: auto` so Cancel and Run stay reachable.

6. `ui/src/modules/workflows/WorkflowsPage.svelte:1322`, menu item `:596`, layout `:2198`. “Inspector on the side” is offered on a phone and applies `padding-inline-end` plus a panel width of ~400px. The graph is covered or pushed out of a ~390px width. **Fix:** force the bottom inspector below 1024px. Hide or disable that menu row on phone and tablet, and do not apply the side width.

7. `ui/src/modules/workflows/WorkflowsPage.svelte:2573` and `.insp-h` at `:4325`. A long node name does not shrink. “From here” and “Only this” slide out of the inspector and need a horizontal scroll. **Fix:** `min-width: 0` and ellipsis on the name (with `title` set to the full name). Let the header wrap so the two run buttons stay on the next line under 640px.

8. `ui/src/modules/workflows/WorkflowsPage.svelte:4027`. Running-list names are `max-width: 120px` while `.grow` eats the rest of a widened pane. A long workflow name stays truncated even when the list is wide. The row’s `title` has the full string, but the visible name never grows. **Fix:** remove the 120px cap; `flex: 1; min-width: 0` with ellipsis so the name takes the space before the id, progress, and time.

9. `ui/src/modules/workflows/RunSteps.svelte:340` and `:360`. “Zoom in on this step” is a `<button>` inside `<summary>`. That is invalid interactive content, so activating it also toggles the step (or the control is skipped). Checkpoint paging at `:326` is a bare browser button, with no busy label while `checkpointLoading`. **Fix:** move zoom to a sibling of `<summary>` in the step row. Use `.btn.small` for Previous / More, and set the label to `Loading…` while the page request is in flight.

10. `ui/src/modules/workflows/RunAgents.svelte:365` and `:301`, and `ui/src/modules/workflows/TriggersPanel.svelte:241`. In the 240px context panel the session row does not shrink (`s-title` is `max-width: 50%` plus provider, status, and id), so the status scrolls off. Sub-agent lines start with a `└` glyph and the raw words `running` / `done` / `failed`. While editing a trigger, Kind is `disabled` with no `title`, so the control looks broken. **Fix:** `min-width: 0` on the session title and let the row wrap; drop the glyph and use the same sentence-case status as `runStatus`. On the Kind select set `title="Kind can’t be changed. Delete this trigger and add a new one."`

## Read, no defect filed

- List load, failed load with Retry, empty page vs empty rail, no-workspace empty: `WorkflowsPage.svelte` `load` (`:621`), list (`:2147`), main (`:3536`). A failed load is not shown as “No workflows yet”.
- Triggers, versions, and runs popover: `LoadState` plus Retry. Trigger delete uses `confirmer` (`TriggersPanel.svelte:176`). Stop run uses `confirmer` (`WorkflowsPage.svelte:1296`). No `confirm()` / `prompt()` / `alert()`.
- Approval copy names the account, project or space, and who can see it (`:2311`). Publication preview has its own loading and retry (`:2312`).
- Header overflow and row ⋯ go through `ctxMenu` (`:570`, `:580`). Node and Save are in that menu on a phone (`:583`). One `PageHeader`, phone back button (`:2026`).
- Canvas titles ellipsis (`WorkflowCanvas.svelte:696`). Workflow row names ellipsis (`WorkflowsPage.svelte:3938`). Run-dot pulse respects `prefers-reduced-motion` (`WorkflowCanvas.svelte:736`, `WorkflowsPage.svelte:4742`). Focus rings exist on nodes, ports, and edges (`WorkflowCanvas.svelte:548`, `:580`, `:769`).
- Module CSS has no hex literals. The hex the user sees is the catalog string applied in defect 1.
- `retryPolicy.ts`, `triggerForm.ts`, `runInput.ts`, `runProgress.ts`, `reviewAgents.ts` are logic only; no UI defect there.
