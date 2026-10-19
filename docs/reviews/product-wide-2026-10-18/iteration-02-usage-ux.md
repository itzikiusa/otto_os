# Usage UX closure — iteration 2

Root owns this narrow follow-up in UsagePage.svelte and ModelBreakdown.svelte;
Ronaldo's existing storage validation, budgets and OttoUsage error repairs remain.
No backend changes, Cargo runs, new sessions or budget baseline changes.

## Changes

- Daily and model charts expose selected date, tokens, estimated cost and series
  in HTML. Bar click/focus/Space and labelled Day selectors select the detail.
  The model legend is available to assistive technology.
- SVG viewBox widths follow measured container widths. Date labels use a minimum
  52-pixel spacing instead of a fixed eight-label count; hit overlays remain
  transparent. Model stack order now agrees with the daily chart and legend.
- Provider/feature costs remain visible in a second phone row.
- Empty overview replaces zero KPI/chart panels with one explanatory empty state;
  budgets remain available. System metrics remain independent of usage data.
- Opening storage settings makes Save budgets secondary. CPU/memory sparklines
  preserve their aspect ratio.

These address the five Usage findings in design-grok/10 and chart/cost findings
1–3 in design-grok/16. Existing budget wrapping and OttoUsage error handling are
separate prior repairs; this pass does not claim new verification of every
accounting/storage behavior or an overall 9.8 score.

## Verification

- ui/unit/usageChartLayout.test.ts: two production-function spacing regressions
  failed before the change and passed after it; widths260/390/700 and7/30/90days.
  Logs: /tmp/otto-usage-chart-layout-{red,green}.log.
- Named Playwright desktop-usage-closure.spec.ts: **2 passed, zero failed,4.3s**,
  light/dark390x844. Checks bar click, focus/Space selection, explicit Day selection,
  exact HTML detail, visible provider/feature costs, non-shrunken chart coordinates,
  no horizontal page overflow, and empty state retaining Budgets.
  Log: /tmp/otto-usage-closure-browser.log. Isolated slot22, routes mock usage
  data; this proves UI behavior, not current accounting backend execution.
- Svelte check **zero errors/warnings**, E2E TypeScript check exit0:
  /tmp/otto-usage-svelte-final.log. Scoped diff whitespace clean.
- Light/dark captures: test-results/usage-phone-{light,dark}.png. Dark screenshot
  visually inspected: readable axes/caption, cost visible, no clipped card width.
- Full npm check initially stopped at concurrent Swarm guard violations
  (KanbanBoard global-class and RunInspector text loader); sent to Pele.
  This is not recorded as a passing combined gate.

A first added native-select ArrowDown automation assertion did not change the
macOS Chromium popup value. The final test exercises our SVG focus/Space behavior
and native select through Playwright selectOption; no platform-wide native-popup
keyboard claim is made. No production code was changed to accommodate that probe.

Final bundle follow-up: the combined build succeeded but page:usage exceeded
the unchanged budget (37.4KB vs36.1KB). UsageReportView now loads only when its
Report tab is opened, with loading and Retry states. The extended named browser
suite verifies that the Report tab mounts and displays its fetched empty state
in both light/dark phone cases:2passed,0failed,7.5s,
/tmp/otto-usage-lazy-browser.log. Remeasured bundle result remains pending the
combined build. Eight chart/report unit cases passed before the lazy-load change.
Light/dark screenshots are also preserved under this report directory's
screenshots/usage-phone-{light,dark}.png.
