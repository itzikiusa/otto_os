# Design review — Usage, Insights, Skills Lab, Skills Eval, Plugins

Reviewer: Grok (design), 2026-10-18. Read-only pass over `ui/src/modules/usage/`, `insights/`, `skills-lab/`, `skills-eval/`, and `plugins/`, against `docs/design/guidelines/` and `ui/src/lib/tokens.css`. No Cargo, npm, or browser run. Flows traced in source: loading / empty / error / loaded, light and dark token use, phone (≤640) and tablet (≤1024).

Scores are not inflated to the 9.8 target.

| Module | Score | Defects |
|---|---|---|
| Usage | 8.6 | 5 |
| Insights | 9.1 | 2 |
| Skills Lab | 8.7 | 3 |
| Skills Eval | 8.4 | 3 |
| Plugins | 9.0 | 2 |
| **Total** | **8.8 unweighted** | **15** |

## Usage — 8.6

Overview, Report, and Otto usage share `PageHeader`, semantic tones, and `--cat-*` series. The not-installed page is one empty state plus a secondary binary path. Report tables scroll inside the card. Phone stacks the four stat cards and the two-column grid.

1. **Empty overview is a wall of zero tiles, not an empty state.** `UsagePage.svelte:681-722` (and the “No usage in this window” lines at 786, 873, 915, 1254) still render four KPI cards, token panels, budgets, and system load when `summary` exists and every total is 0. The Report tab already does the right thing at `UsageReportView.svelte:68-69`. **Impact:** a new or quiet window looks broken (a page of “0” and repeated “no usage” sentences) instead of one next step. **Fix:** when `total_tokens === 0` and there are no sessions, render the same `EmptyState` as the report (“No usage in this window”, longer window / All) and keep the budgets editor beneath it for root. Do not render the zero cards.

2. **Phone drops estimated cost off the bar charts.** `UsagePage.svelte:1934-1936` sets `.bar-row .bar-cost { display: none }` and the comment says cost remains in the hover title. There is no hover on a phone, and the sessions table on the same page keeps its cost column (`1957-1971`). **Impact:** provider and feature rows, the place cost is compared, show tokens only. **Fix:** keep cost on a second line under the name (or beside the token count). Do not rely on `<title>`.

3. **Daily and model charts fight the pointer and the phone width.** The hit target is a full-height column that turns opaque on hover (`UsagePage.svelte:848-856` with `.bar-hit:hover` at `1532-1538`; same pattern `ModelBreakdown.svelte:152` and `266-272`), so the stack colours disappear while you read the tooltip. Axis labels use `font-size: var(--fs-xs)` (`1521-1524`, `ModelBreakdown.svelte:261-264`), which is 11 CSS px inside a viewBox that scales down; on a phone the y labels clip at the card edge (`.usage` is `overflow: hidden`, `1274-1278`) and “Sep 16” labels overlap when `showXLabel` still draws every day for windows of ≤8 (`335-339`). **Impact:** the chart is hardest to read at the moment and width you need it. **Fix:** leave the hit rect transparent (stroke or a 1px halo only). Set SVG `font-size` in user units sized for the viewBox, and thin x labels from the rendered bar width, not from a fixed count of 8.

4. **Model stack order does not match the legend.** `ModelBreakdown.svelte:139-144` anchors the first series at the bottom (`plotH - cumulative`). The legend (`124-127`) lists that series first, at the top. The daily chart stacks the other way (first category at the top of the bar, `UsagePage.svelte:829-840`), and its own comment still says “bottom = input”. **Impact:** the largest model looks like the bottom band, not the swatch at the top of the legend. **Fix:** one stack direction everywhere, matching legend order (first swatch at the bottom, or first at the top). Delete the stale comment.

5. **Two primary saves, and the CPU sparkline is stretched.** Storage “Save settings” (`UsagePage.svelte:633-645`) and “Save budgets” (`1119-1126`) are both `.btn.primary` and can be open together. System sparklines use `preserveAspectRatio="none"` (`1149-1156`) on a 300×48 viewBox, so a CPU spike is smeared across the card width. Otto usage failures are the raw `e.message` plus “Dismiss error” (`OttoUsage.svelte:80`, `163`), with no retry. **Impact:** two accent buttons compete; the sparkline shape is not the data; a failed telemetry load cannot be retried in place. **Fix:** one primary (the settings save, or the budgets save while that editor is open). Use the default `meet` aspect on the sparks. Route Otto usage errors through `loadErrorText` and a Retry control.

## Insights — 9.1

Reports are a real list/detail: phone push with a header back button (`InsightsPage.svelte:677-681`, `1049-1055`), empty and error states, filter chips that hide zero counts, and a preview that attributes the agent (`ReportDetail.svelte:385`). KPI deltas pair colour with an arrow and a screen-reader string (`294-297`, `174-180`). Health has loading, error, empty, and a per-row fix. Tablet keeps the split, which still fits.

1. **The loading skeleton overflows the phone sideways.** `InsightsPage.svelte:746-749` renders list and detail together, and neither has `hide-phone`. At `1049-1052` the list becomes `width: 100%` while the detail stays `flex: 1` in a row (`901-905`). **Impact:** the first paint of Insights on a phone is wider than the screen; the report skeleton sits off to the side. **Fix:** on phone, show only the list skeleton until `phoneDetail` (same `hide-phone` rule as the loaded split).

2. **The period controls are a 22 px target.** Filter chips are `height: 22px` (`928-940`). The “Run now” period `<select>` is `height: 22px` (`863-867`), under `--hit-min` (36 px), and it sits in the header next to the primary action. **Impact:** on a phone, changing Daily/Weekly or the run period is a mis-tap. **Fix:** at `max-width: 640px`, give both `min-block-size: var(--hit-min)` and keep the visual chip/select from growing the type size.

## Skills Lab — 8.7

Skills themselves are push navigation on a phone (`SkillsBrowser.svelte:369`, `455`, `734-743`) with a header back button (`SkillsLabPage.svelte:95-97`). Empty library, load error, filters, and the file editor’s container query (`SkillEditor.svelte:435-447`) are in good shape. Copy is sentence case. Sync dots are paired with text in the row’s accessible name.

1. **Review is not push navigation.** `SkillReviewPanel.svelte:695-700` stacks the list at `max-height: 40%` over the form. `SkillsLabPage.svelte:95` only shows Back on the Skills tab, and the review toolbar’s show/hide list control (`SkillReviewPanel.svelte:360`) does not replace that. **Impact:** on a phone the new-review form and the findings are stuck under a list that eats almost half the height; there is no way to focus the form. **Fix:** same pattern as Skills: hide the list when a review or the form is open, and put Back in `PageHeader` `leading`.

2. **Skills loading ignores the phone split.** `SkillsBrowser.svelte:348-352` always mounts list and detail skeletons. `hide-phone` applies only after data (`734-739`). **Impact:** opening Skills Lab on a phone scrolls horizontally until the list returns. **Fix:** while `loading`, render a single page skeleton on `viewport.isPhone`.

3. **Two primaries on a skill that is not in the library.** The page primary is “New skill” (`SkillsLabPage.svelte:140`). The detail also uses `.btn.small.primary` for “Install to library” / “Update library copy” / “Copy to library” (`SkillDetail.svelte:304-307`). **Impact:** the accent button in the header and the accent button on the skill compete; the one that changes this skill is not obviously the action. **Fix:** keep New skill as the only `.primary`. Make install/copy a normal `.btn`.

## Skills Eval — 8.4

Runs, golden tasks, and matrix each have loading, empty, error, and a single primary when the list is empty. Compare uses score text plus a check, not colour alone (`CompareView.svelte:59-82`). Proof and promote go through a modal that names the overwrite. Reduced motion stops the running dot (`SkillsEvalPage.svelte:542-545`).

1. **Runs and Matrix never become a phone page.** `SkillsEvalPage.svelte:522-531` and `MatrixView.svelte:617-625` keep a 40% list stacked on the form, run, or grid. The Lab back button does not cover Evaluator (`SkillsLabPage.svelte:91-97`). **Impact:** starting an evaluation or reading a matrix on a phone means working in the bottom half of the screen, under a list you cannot dismiss. **Fix:** push navigation. Selecting a run, opening the form, or opening a matrix hides the list and sets a Back control in the Lab header (extend `phoneDetail` beyond the Skills tab).

2. **Golden tasks show two primaries while creating.** The header “New golden task” stays `.primary` when the form is open (`GoldenTasksView.svelte:207-216`, condition includes `showForm`), and the form submit is also `.primary` (`282`). **Impact:** “New” and “Add” are both the accent action; Cancel is the quiet one. **Fix:** while `showForm`, drop `.primary` from the header button (or hide it). The submit stays the only primary.

3. **The run header is a bad phone row.** Rating stars are 24×24 (`RunDetail.svelte:782-793`), below `--hit-min`. In the same header, “Promote winning skill” (`.primary`, `470-473`) sits directly against Delete (`475-477`); Stop is in that row too (`463-468`). **Impact:** rating is a mis-tap, and the destructive control is adjacent to the primary, which the header rules forbid. **Fix:** on phone, `min-width/height: var(--hit-min)` on `.star` (keep the 14 px icon). Move Delete into the existing overflow or a confirm menu, not beside Promote.

## Plugins — 9.0

`PluginFrame` uses `PageHeader`, probes before mounting, and separates missing (empty state, settings only for root) from a failed load (inline error and Retry) (`PluginFrame.svelte:188-229`). Theme changes re-post computed colours (`125-137`). The iframe title is the plugin name.

1. **Escape never leaves the frame.** The host only re-dispatches `otto:keydown` (`PluginFrame.svelte:139-163`). The contract (`7-10`) and the reference listener (`examples/plugins/team-performance/ui/views/app.js:85-90`) forward modifier chords only, so Escape is dropped. **Impact:** with focus in a plugin field, Esc does not close Otto’s top layer (palette, menu, confirm), and Shift-Tab does not return to the header. **Fix:** treat Escape as a chord the shell always receives: the reference listener posts it with no modifier, and `PluginFrame` keeps re-dispatching it. On the iframe’s first focusable boundary, post a `otto:focus-leave` so the host focuses Restart / Settings.

2. **Direction is not part of init or theme.** `onload` and the theme effect (`109-121`, `129-136`) send colours and `scheme` but not `dir` or `themeName`. The plugin document does not inherit `documentElement.dir`. **Impact:** in an RTL Otto the plugin page stays LTR (icons and rows do not mirror) even though the header does. Named-theme fallbacks in the plugin (`data-otto-theme`) also never activate for any token the host did not inline. **Fix:** add `dir: ui.direction` and `themeName: ui.theme` to both messages, and set `frame.contentDocument.documentElement.dir` on load (same origin). Re-send on direction change the same way theme already re-sends.

## Residual (not scored as defects)

- Usage, Insights, Skills, and Eval module CSS did not introduce hex, sub-11 px type, native `confirm`/`prompt`, or undefined tokens. `--text-dim` on `--surface-2` / `--surface-3` matches the current foundations contrast table.
- Insight HTML is agent-authored inside a sandboxed iframe (`ReportDetail.svelte:266-274`). Preview is themed; the HTML document’s own colours were not inspected because there is no fixed template in these modules.
- Tablet (641–1024) stacks Usage’s card grid to two columns and the chart grid to one. Insights and Skills stay side-by-side. That is acceptable at 768 px; no separate tablet defect.
- Plugin chart colours (`--cat-*`) are not in the host payload. The reference plugin still defines them from `data-theme`, which the host does set via `scheme`, so this was not counted on top of defect 2.
