# Usage and Insights — design review

Reviewed `ui/src/modules/usage/` and `ui/src/modules/insights/` against `docs/design/guidelines/` (layout, components, content, accessibility) and `ui/src/lib/tokens.css`. Code review only: no app screenshots, no Cargo, no npm. Light and dark both use the measured semantic tokens (`--text`, `--warning`, `--success`, `--danger-soft`, `--cat-*` for chart fills). Phone is `max-width: 640px`; tablet is `max-width: 1024px`.

| Surface | Score | Defects |
|---|---|---|
| Usage | 9.0 | 5 |
| Insights | 9.1 | 4 |
| **Total** | | **9** |

Usage is not 9.8 because the daily and model charts, and the budget editor, fail on a phone. Insights is not 9.8 because a failed or in-progress run mislabels itself and can push Stop off the screen. Empty usage, failed loads on first paint, and the report list/detail states are in good shape (see the end).

## Usage

Every item below is **RONALDO-EDIT**. Do not fix it in this session.

### RONALDO-EDIT 1. Daily and model charts only expose a day’s numbers in an SVG `<title>`

`ui/src/modules/usage/UsagePage.svelte:848-856`
`ui/src/modules/usage/ModelBreakdown.svelte:152`

The hit target’s `<title>` is the only place a bar’s date, token total, cost, and segment split are written out. `title` is a hover tooltip. On a phone or tablet it does not appear on tap, so the chart is a shape with no readable values. The model chart’s legend is also `aria-hidden` (`ModelBreakdown.svelte:124`), and the chart’s `aria-label` does not name the series, so the same bars are unnamed for VoiceOver.

**Fix:** On click or keyboard focus of a bar, render an HTML caption under the SVG (date, total tokens, cost, and each series with its swatch and count). Keep `<title>` as a duplicate, not the only copy. Include the series names in the chart’s accessible name, or stop hiding the legend from assistive tech.

### RONALDO-EDIT 2. X-axis labels collide on a narrow chart

`ui/src/modules/usage/UsagePage.svelte:336-340` (drawn at `859-865`)
`ui/src/modules/usage/ModelBreakdown.svelte:76-80` (drawn at `153-155`)

`showXLabel` / `showX` always keeps about eight labels, independent of width. Labels are 11 px (`--fs-xs`) and do not scale with the viewBox; “Jun 16” is ~36 px wide. On a phone the plot is ~250 px, so those eight anchors sit ~29 px apart and overlap. The 7-day window (every day labelled) is the same collision. Desktop width is fine.

**Fix:** Measure the SVG (the page already uses container-style layout elsewhere) and skip labels until centres are at least ~48 px apart. Four labels are enough under 640 px.

### RONALDO-EDIT 3. Provider and feature cost disappears on a phone

`ui/src/modules/usage/UsagePage.svelte:1934-1936`

The phone rule sets `.bar-row .bar-cost { display: none }` and the comment says the cost remains in the bar’s hover title. Same problem as defect 1: there is no hover, so “By provider” and “By feature” show tokens only. Cost is the number people open Usage to check.

**Fix:** Keep the cost in the row. Put it under the token count in `.bar-val` (the value cell already has a second line for nothing) and delete the `display: none`.

### RONALDO-EDIT 4. The budget window row overflows the phone

`ui/src/modules/usage/UsagePage.svelte:1026-1041`
`ui/src/modules/usage/UsagePage.svelte:1857-1862`

`.window-row` is a nowrap flex row: the label “Compare spend over the last”, a 96 px input, and “days”. That is ~370 px. Inside the card padding on a 320–390 px phone the row is wider than the page, so the budget editor scrolls sideways. The rest of the editor (caps, save) is below the fold of that scroll.

**Fix:** Allow the row to wrap (`flex-wrap: wrap`) and let the label sit on its own line under 640 px. Give the number input `max-width: 100%`.

### RONALDO-EDIT 5. Otto usage errors show the raw exception

`ui/src/modules/usage/OttoUsage.svelte:80`
`ui/src/modules/usage/OttoUsage.svelte:101` (same pattern at `108`, `115`, `128`, `208`)
`ui/src/modules/usage/OttoUsage.svelte:163`
`ui/src/modules/usage/OttoUsage.svelte:170`

Load and action failures assign `e.message` and the notice at line 163 prints that string as the alert. A daemon or network failure becomes “Failed to fetch” or a JSON body, with “Dismiss error” and no next step. `status.last_error` (line 170) is the same class of string, only partly rescued by the ClickHouse sentence appended after it. The rest of Usage goes through `loadErrorText`.

**Fix:** Pass load failures through `loadErrorText` into `LoadState`. For save, analyze, dismiss, profile, and capture, use `toastError` (human title, cause as the body) instead of a raw `role="alert"`. Keep `last_error` as the dim detail under a fixed sentence: “The local collector reported an error. Check that ClickHouse is installed in Usage → Overview, then retry.”

## Insights

### 1. A run that already started is described as if it never did

`ui/src/modules/insights/InsightsPage.svelte:587-590`
`ui/src/modules/insights/InsightsPage.svelte:723-731`

When polling sees the run disappear without an HTML report, `runFailReason` is set to “ended without publishing… failed or was stopped.” The banner always titles that “Couldn’t start the insights run.” and its only action is “Open skills settings”. The run id is cleared on the line above, so “its session shows what happened” is not a link. Someone whose report failed is sent to Skills, which does not explain the failure. The skills button is right only for a start refusal (`runNow`’s `resp.reason`).

**Fix:** Remember the ended `run_id`. Use two banners. Start refused: title “Couldn’t start the insights run.”, body `resp.reason`, action “Open skills settings” only when the reason is the skill. Run ended: title “The insights run didn’t publish a report.”, body the existing sentence, actions “View live session” (navigate to the saved id) and Dismiss. Do not offer Skills for that case.

### 2. The generating banner overflows on a phone, and Stop goes with it

`ui/src/modules/insights/InsightsPage.svelte:734-741`
`ui/src/modules/insights/InsightsPage.svelte:876-885`

`.banner` is a single nowrap row: icon, a long sentence, elapsed time, “View live session”, and “Stop”. Flex items cannot shrink below their text, and the buttons do not wrap. On a 320–390 px phone that row is wider than the content column, so the page scrolls sideways while a report is generating and Stop sits off-screen. That is the control the user needs if the run is wrong.

**Fix:** Under 640 px, stack the banner (`flex-direction: column; align-items: stretch`). Give the message `min-width: 0`. Put the elapsed time and the two buttons on a second row that wraps. Do the same for `.banner.warn`, whose “Open skills settings” button has the same problem.

### 3. The elapsed clock freezes while the window is in the background

`ui/src/modules/insights/InsightsPage.svelte:556-560`
`ui/src/modules/insights/InsightsPage.svelte:738`

The banner shows `pollBaseSec + pollCount * 3`. A hidden document reschedules the poll without incrementing `pollCount` (so the 5-minute cap is not burned, which is correct). The clock uses that same counter, so it pauses while Otto is in the background and then continues from the paused value. A ten-minute run can still read `1:00`. The checklist requires honest progress for live work.

**Fix:** Store `startedAt` (already available as `r.started_at` on restore) and render `Date.now() - startedAt`. Keep `pollCount` only as the give-up counter.

### 4. A failed reload is invisible once any report is on screen

`ui/src/modules/insights/InsightsPage.svelte:142-146`
`ui/src/modules/insights/InsightsPage.svelte:546-551`
`ui/src/modules/insights/InsightsPage.svelte:751-753`

`load()` sets `loadError`, but the error `LoadState` renders only when `reports.length === 0`. The poll’s final `load()` runs when reports are already listed. If that request fails, the old list stays up, `loadError` is set, and there is no Retry. The list looks current.

**Fix:** When `loadError` is set and `reports.length > 0`, show a slim alert above the split: “Couldn’t refresh insight reports.” plus the `loadError` text and Retry. Leave the stale list in place (`LoadState`’s stale-data pattern).

## What held

Usage separates “status failed” (`LoadState`) from “ClickHouse not installed” (`EmptyState` with one Install primary). Budgets, the report tables, and attribution each have their own loading, empty, and error states, and wide tables scroll inside `.tbl-scroll` / `.sess-scroll` rather than the page. Session columns drop workspace and last active on a phone instead of forcing a 680 px page. Chart fills use `--cat-*`, which are specified as graphics-only; category meaning is repeated in the breakdown list and the legend on desktop. The exported HTML report (`reportHtml.ts`) is a standalone file with its own `prefers-color-scheme` palette, not app chrome.

Insights opens on a report, pushes list/detail on a phone with a header back button, attributes the write-up to the `insights` skill, confirms Stop, and keeps Preview / Markdown / HTML plus the HTML load error (human sentence, raw detail dim, Retry). Health uses `LoadState` and `StatusBadge`, and the capability rows wrap under 640 px. KPI deltas pair colour with an arrow and a screen-reader sentence. No hex in either module’s Svelte styles, no undefined tokens, no native `confirm` / `prompt`.
